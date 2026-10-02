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

thread_local! {
    /// Per-thread state-directory override (031-hosted-server-mode). The hosted server runs one
    /// backend per signed-in user in the same process, each with its own state root, so the
    /// override is scoped to that backend's threads (its `fathom-backend` thread plus every
    /// worker and blocking thread of its Tokio runtime, see [`scope_runtime_to_state_root`]).
    /// Desktop and devbridge never set it, so their paths stay `~/.fathom`.
    static THREAD_STATE_ROOT: std::cell::RefCell<Option<PathBuf>> =
        const { std::cell::RefCell::new(None) };
}

/// Set once by a multi-tenant process (the hosted server): a thread with no scoped state root
/// must fail instead of falling back to a directory shared by every user.
static REQUIRE_SCOPED_ROOT: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Install the state-directory override for the current thread.
pub fn set_thread_state_root(root: PathBuf) {
    THREAD_STATE_ROOT.with(|r| *r.borrow_mut() = Some(root));
}

/// Make every thread `builder`'s runtime starts (workers and the blocking pool) use `root`.
pub fn scope_runtime_to_state_root(builder: &mut tokio::runtime::Builder, root: PathBuf) {
    builder.on_thread_start(move || set_thread_state_root(root.clone()));
}

/// Refuse the `~/.fathom` fallback process-wide (hosted server). Irreversible.
pub fn require_scoped_state_root() {
    REQUIRE_SCOPED_ROOT.store(true, std::sync::atomic::Ordering::Release);
}

/// `override_root` if given, else `<home>/.fathom`, unless `require_scoped` forbids the
/// shared fallback.
fn config_dir_resolved(override_root: Option<&Path>, require_scoped: bool) -> Result<PathBuf> {
    if let Some(root) = override_root {
        return Ok(root.to_path_buf());
    }
    if require_scoped {
        return Err(anyhow!(
            "no per-user state directory on this thread; refusing the shared fallback"
        ));
    }
    let home = dirs::home_dir().ok_or_else(|| anyhow!("Could not determine home directory"))?;
    Ok(home.join(".fathom"))
}

/// Get the Fathom config directory (~/.fathom/, or this backend's hosted state root)
pub fn fathom_config_dir() -> Result<PathBuf> {
    let scoped = THREAD_STATE_ROOT.with(|r| r.borrow().clone());
    config_dir_resolved(
        scoped.as_deref(),
        REQUIRE_SCOPED_ROOT.load(std::sync::atomic::Ordering::Acquire),
    )
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

/// Get the path to `audit-log.json` (026-mutation-audit-log-viewer: the local, append-only,
/// bounded record of completed mutating actions). See `src/backend/audit_log.rs`.
pub fn audit_log_path() -> Result<PathBuf> {
    Ok(fathom_config_dir()?.join("audit-log.json"))
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

/// Per-context exec policies parsed from a `config.json` value (035). Absent or invalid yields
/// an empty map (every context is `allowKnownOnly`); an invalid blob is moved to
/// `cloud_auth_invalid` so the original is preserved (Principle VI). Returns whether `config`
/// was modified and must be saved.
pub fn exec_policies_from_config(
    config: &mut serde_json::Value,
) -> (
    std::collections::HashMap<String, crate::protocol::ExecPolicy>,
    bool,
) {
    let Some(raw) = config.get("cloud_auth").cloned() else {
        return (Default::default(), false);
    };
    let parsed = raw
        .get("exec_policies")
        .cloned()
        .map_or(Ok(Default::default()), serde_json::from_value);
    match parsed {
        Ok(policies) => (policies, false),
        Err(e) => {
            tracing::warn!(
                "Invalid cloud_auth config; moving it to cloud_auth_invalid: {}",
                e
            );
            if let Some(obj) = config.as_object_mut() {
                obj.remove("cloud_auth");
                obj.insert("cloud_auth_invalid".to_owned(), raw);
            }
            (Default::default(), true)
        }
    }
}

/// Set (or, for the default policy, remove) one context's policy in a `config.json` value.
pub fn set_exec_policy_in_config(
    config: &mut serde_json::Value,
    context: &str,
    policy: crate::protocol::ExecPolicy,
) -> Result<()> {
    let obj = config
        .as_object_mut()
        .ok_or_else(|| anyhow!("config.json is not a JSON object"))?;
    let section = obj
        .entry("cloud_auth")
        .or_insert_with(|| serde_json::json!({}));
    if !section.is_object() {
        *section = serde_json::json!({});
    }
    let policies = section
        .as_object_mut()
        .expect("just ensured object")
        .entry("exec_policies")
        .or_insert_with(|| serde_json::json!({}));
    if !policies.is_object() {
        *policies = serde_json::json!({});
    }
    let map = policies.as_object_mut().expect("just ensured object");
    if policy == crate::protocol::ExecPolicy::default() {
        map.remove(context);
    } else {
        map.insert(context.to_owned(), serde_json::to_value(policy)?);
    }
    Ok(())
}

/// Load `config.json -> cloud_auth.exec_policies` (035); defaults on any problem.
pub fn load_exec_policies() -> std::collections::HashMap<String, crate::protocol::ExecPolicy> {
    let mut config = match load_config() {
        Ok(config) => config,
        Err(e) => {
            tracing::warn!(
                "Failed to read config.json; using exec policy defaults: {}",
                e
            );
            return Default::default();
        }
    };
    let (policies, changed) = exec_policies_from_config(&mut config);
    if changed && let Err(e) = save_config(&config) {
        tracing::warn!("Failed to move invalid cloud_auth config aside: {}", e);
    }
    policies
}

/// Persist one context's exec policy, leaving every other key untouched. Refuses to write over
/// an unparseable `config.json`.
pub fn save_exec_policy(context: &str, policy: crate::protocol::ExecPolicy) -> Result<()> {
    let mut config = load_config()?;
    set_exec_policy_in_config(&mut config, context, policy)?;
    save_config(&config)
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
    fn state_root_override_and_default() {
        let custom = std::path::Path::new("/var/lib/fathom");
        assert_eq!(config_dir_resolved(Some(custom), false).unwrap(), custom);
        assert_eq!(config_dir_resolved(Some(custom), true).unwrap(), custom);
        let default = config_dir_resolved(None, false).unwrap();
        assert!(default.ends_with(".fathom"));
        // A multi-tenant process never falls back to the shared directory.
        assert!(config_dir_resolved(None, true).is_err());
        // No test installs an override on this thread, so the public path is the default.
        assert_eq!(fathom_config_dir().unwrap(), default);
    }

    #[test]
    fn state_root_is_scoped_to_its_threads() {
        let a = std::env::temp_dir().join("fathom-root-a");
        let b = std::env::temp_dir().join("fathom-root-b");
        let (a2, b2) = (a.clone(), b.clone());
        let got_a = std::thread::spawn(move || {
            set_thread_state_root(a2);
            fathom_config_dir().unwrap()
        })
        .join()
        .unwrap();
        let got_b = std::thread::spawn(move || {
            set_thread_state_root(b2);
            fathom_config_dir().unwrap()
        })
        .join()
        .unwrap();
        assert_eq!(got_a, a);
        assert_eq!(got_b, b);
        // Neither leaked onto this thread.
        assert!(fathom_config_dir().unwrap().ends_with(".fathom"));

        // Every thread of a scoped runtime (workers and blocking pool) sees the root.
        let mut builder = tokio::runtime::Builder::new_multi_thread();
        builder.worker_threads(2).enable_all();
        scope_runtime_to_state_root(&mut builder, a.clone());
        let rt = builder.build().unwrap();
        let (worker, blocking) = rt.block_on(async {
            let w = tokio::spawn(async { fathom_config_dir().unwrap() })
                .await
                .unwrap();
            let b = tokio::task::spawn_blocking(|| fathom_config_dir().unwrap())
                .await
                .unwrap();
            (w, b)
        });
        assert_eq!(worker, a);
        assert_eq!(blocking, a);
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

/// 034-simultaneous-federation: `~/.fathom/federation.json`. Context names only; no secrets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FederationFile {
    #[serde(default = "federation_file_version")]
    pub version: u32,
    #[serde(default)]
    pub contexts: Vec<String>,
    #[serde(default = "federation_soft_cap")]
    pub soft_cap: u32,
}

fn federation_file_version() -> u32 {
    1
}

fn federation_soft_cap() -> u32 {
    crate::protocol::FEDERATION_SOFT_CAP
}

impl Default for FederationFile {
    fn default() -> Self {
        Self {
            version: 1,
            contexts: Vec::new(),
            soft_cap: federation_soft_cap(),
        }
    }
}

/// Path of `federation.json`.
pub fn federation_config_path() -> Result<PathBuf> {
    Ok(fathom_config_dir()?.join("federation.json"))
}

/// Load the federated set from `path`. Absent yields empty. A file that cannot be parsed
/// yields empty and is preserved as `federation.json.corrupt` (never silently overwritten).
pub fn load_federation_from(path: &Path) -> FederationFile {
    let Ok(content) = fs::read_to_string(path) else {
        return FederationFile::default();
    };
    match serde_json::from_str::<FederationFile>(&content) {
        Ok(file) => file,
        Err(e) => {
            tracing::warn!("Failed to parse {}: {}; using empty set", path.display(), e);
            let mut backup = path.as_os_str().to_owned();
            backup.push(".corrupt");
            if let Err(e) = fs::copy(path, PathBuf::from(backup)) {
                tracing::warn!("Failed to preserve corrupt federation file: {}", e);
            }
            FederationFile::default()
        }
    }
}

/// Atomically write the federated set to `path` (temp file + rename).
pub fn save_federation_to(path: &Path, file: &FederationFile) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    fs::write(&tmp, serde_json::to_string_pretty(file)?)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

pub fn load_federation() -> FederationFile {
    federation_config_path()
        .map(|p| load_federation_from(&p))
        .unwrap_or_default()
}

pub fn save_federation(file: &FederationFile) -> Result<()> {
    save_federation_to(&federation_config_path()?, file)
}

#[cfg(test)]
mod federation_tests {
    use super::*;

    #[test]
    fn roundtrip_and_absent_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("federation.json");
        assert!(load_federation_from(&path).contexts.is_empty());
        let file = FederationFile {
            contexts: vec!["a".into(), "b".into()],
            ..FederationFile::default()
        };
        save_federation_to(&path, &file).unwrap();
        assert_eq!(load_federation_from(&path), file);
        assert!(!dir.path().join("federation.json.tmp").exists());
    }

    #[test]
    fn corrupt_file_falls_back_and_is_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("federation.json");
        fs::write(&path, "{not json").unwrap();
        assert!(load_federation_from(&path).contexts.is_empty());
        assert_eq!(
            fs::read_to_string(dir.path().join("federation.json.corrupt")).unwrap(),
            "{not json"
        );
    }

    #[test]
    fn unknown_keys_and_missing_fields_are_tolerated() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("federation.json");
        fs::write(&path, r#"{"contexts":["x"],"future":1}"#).unwrap();
        let f = load_federation_from(&path);
        assert_eq!(f.contexts, vec!["x".to_owned()]);
        assert_eq!(f.soft_cap, 8);
    }
}

#[cfg(test)]
mod exec_policy_tests {
    use super::*;
    use crate::protocol::ExecPolicy;
    use serde_json::json;

    #[test]
    fn absent_config_defaults_and_is_unchanged() {
        let mut c = json!({"ui": {"a": 1}});
        let (p, changed) = exec_policies_from_config(&mut c);
        assert!(p.is_empty() && !changed);
    }

    #[test]
    fn set_is_additive_and_default_removes_entry() {
        let mut c = json!({"ui": {"a": 1}});
        set_exec_policy_in_config(&mut c, "dev", ExecPolicy::AllowAll).unwrap();
        assert_eq!(c["ui"]["a"], 1);
        assert_eq!(c["cloud_auth"]["exec_policies"]["dev"], "allowAll");
        let (p, _) = exec_policies_from_config(&mut c);
        assert_eq!(p.get("dev"), Some(&ExecPolicy::AllowAll));
        set_exec_policy_in_config(&mut c, "dev", ExecPolicy::AllowKnownOnly).unwrap();
        assert!(exec_policies_from_config(&mut c).0.is_empty());
    }

    #[test]
    fn invalid_blob_yields_defaults_and_is_preserved() {
        let mut c = json!({"cloud_auth": {"exec_policies": {"dev": "bogus"}}});
        let (p, changed) = exec_policies_from_config(&mut c);
        assert!(p.is_empty() && changed);
        assert!(c.get("cloud_auth").is_none());
        assert_eq!(c["cloud_auth_invalid"]["exec_policies"]["dev"], "bogus");
    }
}

/// 036-incluster-daemon: `~/.fathom/daemon.json` (additive). Per-context preference and the
/// last event cursor so the timeline resumes across restarts. No tokens or secrets.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DaemonFile {
    /// Missing entry means `DaemonMode::Off`.
    #[serde(default)]
    pub preferences: BTreeMap<String, crate::protocol::DaemonMode>,
    #[serde(default)]
    pub cursors: BTreeMap<String, crate::protocol::EventCursor>,
}

impl DaemonFile {
    #[must_use]
    pub fn mode_for(&self, context: &str) -> crate::protocol::DaemonMode {
        self.preferences.get(context).copied().unwrap_or_default()
    }
}

/// Path of `daemon.json`.
pub fn daemon_config_path() -> Result<PathBuf> {
    Ok(fathom_config_dir()?.join("daemon.json"))
}

/// Load `daemon.json` from `path`. Absent or unparseable yields defaults; an unparseable file
/// is left untouched (copied to `.corrupt`, never deleted or overwritten here) — Principle VI.
pub fn load_daemon_from(path: &Path) -> DaemonFile {
    let Ok(content) = fs::read_to_string(path) else {
        return DaemonFile::default();
    };
    serde_json::from_str::<DaemonFile>(&content).unwrap_or_else(|e| {
        tracing::warn!("Failed to parse {}: {}; using defaults", path.display(), e);
        let mut backup = path.as_os_str().to_owned();
        backup.push(".corrupt");
        if let Err(e) = fs::copy(path, PathBuf::from(backup)) {
            tracing::warn!("Failed to preserve corrupt daemon file: {}", e);
        }
        DaemonFile::default()
    })
}

/// Atomically write `daemon.json` (temp file + rename).
pub fn save_daemon_to(path: &Path, file: &DaemonFile) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    fs::write(&tmp, serde_json::to_string_pretty(file)?)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod daemon_file_tests {
    use super::*;
    use crate::protocol::{DaemonMode, EventCursor};

    #[test]
    fn roundtrip_and_default_mode_is_off() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("daemon.json");
        assert_eq!(load_daemon_from(&path), DaemonFile::default());
        let mut f = DaemonFile::default();
        f.preferences.insert("a".into(), DaemonMode::Auto);
        f.cursors.insert(
            "a".into(),
            EventCursor {
                epoch: "e".into(),
                seq: 7,
            },
        );
        save_daemon_to(&path, &f).unwrap();
        let back = load_daemon_from(&path);
        assert_eq!(back, f);
        assert_eq!(back.mode_for("a"), DaemonMode::Auto);
        assert_eq!(back.mode_for("missing"), DaemonMode::Off);
    }

    #[test]
    fn unknown_and_missing_fields_tolerated() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("daemon.json");
        fs::write(&path, r#"{"future":1,"preferences":{"a":"auto"}}"#).unwrap();
        let f = load_daemon_from(&path);
        assert_eq!(f.mode_for("a"), DaemonMode::Auto);
        assert!(f.cursors.is_empty());
    }

    #[test]
    fn corrupt_file_falls_back_without_deleting() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("daemon.json");
        fs::write(&path, "{not json").unwrap();
        assert_eq!(load_daemon_from(&path), DaemonFile::default());
        assert_eq!(fs::read_to_string(&path).unwrap(), "{not json");
        assert!(dir.path().join("daemon.json.corrupt").exists());
    }
}

// ---------------------------------------------------------------------------------------
// 045-guided-oidc-setup (FR-036): which local clusters Fathom provisioned itself.
// ---------------------------------------------------------------------------------------

/// One kind cluster Fathom created, as `~/.fathom/kind-provenance.json` records it.
///
/// This file is the *only* answer to "did Fathom provision this cluster?". The question cannot
/// be asked of `kind get clusters`, which reports every cluster on the machine, nor of the
/// context name, which an operator's own `kind create cluster` produces identically. 045's API
/// server write (FR-019) and its refusal on anything else (FR-022) both turn on it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KindProvenanceRecord {
    /// The cluster name as `kind get clusters` reports it (not the `kind-` context name).
    pub cluster: String,
    /// Unix seconds at which Fathom's provision succeeded.
    pub provisioned_at: i64,
    /// The profile the cluster was created from, which `summaries_from_names` could only
    /// hardcode as `None` before this record existed.
    #[serde(default)]
    pub profile: Option<crate::protocol::KindClusterProfileId>,
}

/// `kind-provenance.json`'s on-disk shape. Cluster names only: no credentials, no kubeconfig.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KindProvenanceFile {
    #[serde(default)]
    pub clusters: Vec<KindProvenanceRecord>,
}

impl KindProvenanceFile {
    /// The record for `cluster`, or `None` — which means *not owned*, whatever the cluster's
    /// context name looks like.
    #[must_use]
    pub fn record_for(&self, cluster: &str) -> Option<&KindProvenanceRecord> {
        self.clusters.iter().find(|r| r.cluster == cluster)
    }

    /// The record for a kubeconfig context, which kind names `kind-<cluster>`.
    ///
    /// The prefix is only how the cluster name is recovered from the context; it is never the
    /// ownership test itself, because a cluster created outside Fathom has an identical one.
    #[must_use]
    pub fn record_for_context(&self, context: &str) -> Option<&KindProvenanceRecord> {
        self.record_for(context.strip_prefix("kind-")?)
    }
}

/// Path of `kind-provenance.json`.
pub fn kind_provenance_path() -> Result<PathBuf> {
    Ok(fathom_config_dir()?.join("kind-provenance.json"))
}

/// Load the provenance record from `path`. Absent or unparseable yields an empty record — which
/// means "Fathom owns nothing", the safe answer — and an unparseable file is preserved as
/// `.corrupt` rather than overwritten (Principle VI).
pub fn load_kind_provenance_from(path: &Path) -> KindProvenanceFile {
    let Ok(content) = fs::read_to_string(path) else {
        return KindProvenanceFile::default();
    };
    serde_json::from_str::<KindProvenanceFile>(&content).unwrap_or_else(|e| {
        tracing::warn!("Failed to parse {}: {}; owning nothing", path.display(), e);
        let mut backup = path.as_os_str().to_owned();
        backup.push(".corrupt");
        if let Err(e) = fs::copy(path, PathBuf::from(backup)) {
            tracing::warn!("Failed to preserve corrupt kind provenance file: {}", e);
        }
        KindProvenanceFile::default()
    })
}

/// Atomically write the provenance record to `path` (temp file + rename).
pub fn save_kind_provenance_to(path: &Path, file: &KindProvenanceFile) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    fs::write(&tmp, serde_json::to_string_pretty(file)?)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

pub fn load_kind_provenance() -> KindProvenanceFile {
    kind_provenance_path()
        .map(|p| load_kind_provenance_from(&p))
        .unwrap_or_default()
}

/// Record that Fathom provisioned `cluster`, in `path`. Called only when a provision succeeded,
/// so the record never claims a cluster Fathom failed to create. Re-recording replaces the
/// previous entry rather than duplicating it.
pub fn record_kind_provision_at(
    path: &Path,
    cluster: &str,
    profile: Option<crate::protocol::KindClusterProfileId>,
    provisioned_at: i64,
) -> Result<()> {
    let mut file = load_kind_provenance_from(path);
    file.clusters.retain(|r| r.cluster != cluster);
    file.clusters.push(KindProvenanceRecord {
        cluster: cluster.to_owned(),
        provisioned_at,
        profile,
    });
    save_kind_provenance_to(path, &file)
}

/// Drop `cluster` from `path`'s record, because Fathom deleted it. A cluster that is gone must
/// not leave behind a record that would make a later same-named cluster look owned.
pub fn forget_kind_provision_at(path: &Path, cluster: &str) -> Result<()> {
    let mut file = load_kind_provenance_from(path);
    if file.record_for(cluster).is_none() {
        return Ok(());
    }
    file.clusters.retain(|r| r.cluster != cluster);
    save_kind_provenance_to(path, &file)
}

/// Unix seconds now, for a provision's timestamp.
#[must_use]
pub fn now_unix_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

pub fn record_kind_provision(
    cluster: &str,
    profile: Option<crate::protocol::KindClusterProfileId>,
) -> Result<()> {
    record_kind_provision_at(&kind_provenance_path()?, cluster, profile, now_unix_secs())
}

pub fn forget_kind_provision(cluster: &str) -> Result<()> {
    forget_kind_provision_at(&kind_provenance_path()?, cluster)
}

#[cfg(test)]
mod kind_provenance_tests {
    use super::*;
    use crate::protocol::KindClusterProfileId;

    /// T056 (FR-036, FR-022): ownership comes from the record and from nothing else. A kind
    /// cluster created outside Fathom has a `kind-`-prefixed context exactly like one Fathom
    /// created, so the context name can never be the test.
    #[test]
    fn a_cluster_without_a_record_is_not_owned() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kind-provenance.json");

        // Nothing recorded yet: nothing is owned, including a plausible-looking context.
        let empty = load_kind_provenance_from(&path);
        assert_eq!(empty, KindProvenanceFile::default());
        assert!(empty.record_for("hand-built").is_none());
        assert!(empty.record_for_context("kind-hand-built").is_none());

        record_kind_provision_at(&path, "fathom-dev", Some(KindClusterProfileId::Bare), 1_700)
            .unwrap();
        let file = load_kind_provenance_from(&path);

        // The one cluster Fathom created is owned, by name and through its context.
        let ours = file.record_for_context("kind-fathom-dev").unwrap();
        assert_eq!(ours.cluster, "fathom-dev");
        assert_eq!(ours.provisioned_at, 1_700);
        assert_eq!(ours.profile, Some(KindClusterProfileId::Bare));

        // A cluster the operator built by hand is not owned, although `kind get clusters` lists
        // it beside ours and its context name is indistinguishable in shape.
        assert!(file.record_for("hand-built").is_none());
        assert!(file.record_for_context("kind-hand-built").is_none());
        // Nor is a non-kind context, which carries no cluster name to look up at all.
        assert!(file.record_for_context("prod-eks").is_none());
    }

    #[test]
    fn re_recording_replaces_and_deleting_forgets() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kind-provenance.json");
        record_kind_provision_at(&path, "dev", Some(KindClusterProfileId::Bare), 1).unwrap();
        record_kind_provision_at(&path, "dev", Some(KindClusterProfileId::CustomYaml), 2).unwrap();
        let file = load_kind_provenance_from(&path);
        assert_eq!(file.clusters.len(), 1);
        assert_eq!(
            file.record_for("dev").unwrap().profile,
            Some(KindClusterProfileId::CustomYaml)
        );

        // Deleting the cluster drops the record, so a later same-named cluster someone else
        // creates is not inherited as owned.
        forget_kind_provision_at(&path, "dev").unwrap();
        assert!(load_kind_provenance_from(&path).record_for("dev").is_none());
        // Forgetting something that was never recorded is not an error.
        forget_kind_provision_at(&path, "dev").unwrap();
    }

    #[test]
    fn unknown_fields_tolerated_and_corrupt_file_owns_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kind-provenance.json");
        fs::write(
            &path,
            r#"{"future":1,"clusters":[{"cluster":"a","provisionedAt":5}]}"#,
        )
        .unwrap();
        let file = load_kind_provenance_from(&path);
        assert_eq!(file.record_for("a").unwrap().provisioned_at, 5);
        assert_eq!(file.record_for("a").unwrap().profile, None);

        fs::write(&path, "{not json").unwrap();
        assert_eq!(
            load_kind_provenance_from(&path),
            KindProvenanceFile::default()
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), "{not json");
        assert!(dir.path().join("kind-provenance.json.corrupt").exists());
    }
}

// ---------------------------------------------------------------------------------------
// 046-fleet-model: `~/.fathom/fleet.json`.
//
// Operator-authored fleet metadata: context names, display names, labels, selector
// expressions and the adopted shared definitions. Nothing else belongs in this file, and
// none of this belongs anywhere else — not `cluster-settings.json`, not `federation.json`,
// not any per-context file (FR-008). It MUST NOT contain a credential, token or secret of
// any kind (FR-020); `fleet_file_holds_no_credential_shaped_value` asserts that.
//
// Schema evolution is **additive only**: a new version adds fields, never renames or
// removes one, and a reader at schema `n` reads a file written at schema `n` while ignoring
// every key it does not know (FR-007). That is why every field below carries
// `#[serde(default)]` and why there is no `deny_unknown_fields` anywhere in this block.
// ---------------------------------------------------------------------------------------

/// One member's persisted metadata. The context name is the map key, so it is not repeated
/// here — `FleetMember::context` is filled in from the key on load.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FleetMemberRecord {
    /// Trimmed; empty-after-trim is stored as absent, never `""`.
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
    /// Per-field provenance, keyed `display_name` or `labels/<key>` (FR-013).
    #[serde(default)]
    pub origins: BTreeMap<String, crate::protocol::SettingOrigin>,
}

/// One group's persisted definition. The group name is the map key.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FleetGroupRecord {
    /// The operator's expression, verbatim as typed (FR-006).
    #[serde(default)]
    pub selector: String,
    #[serde(default)]
    pub origin: crate::protocol::SettingOrigin,
}

/// `fleet.json`'s on-disk shape: `{ schema, members, groups, adopted }`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FleetFile {
    #[serde(default = "fleet_file_version")]
    pub schema: u32,
    /// Keyed by kubeconfig context name, verbatim (FR-001).
    #[serde(default)]
    pub members: BTreeMap<String, FleetMemberRecord>,
    /// Keyed by group name.
    #[serde(default)]
    pub groups: BTreeMap<String, FleetGroupRecord>,
    /// Adopted shared definitions, keyed by definition id. Never deleted because the store
    /// became unreachable or the upstream definition disappeared — only `still_shared`
    /// changes (FR-014).
    #[serde(default)]
    pub adopted: BTreeMap<String, crate::protocol::AdoptedFleetDefinition>,
}

impl Default for FleetFile {
    fn default() -> Self {
        Self {
            schema: fleet_file_version(),
            members: BTreeMap::new(),
            groups: BTreeMap::new(),
            adopted: BTreeMap::new(),
        }
    }
}

/// The schema version this build writes, and the highest it understands. Currently `1`.
#[must_use]
pub fn fleet_file_version() -> u32 {
    1
}

/// Path of `fleet.json`.
pub fn fleet_file_path() -> Result<PathBuf> {
    Ok(fathom_config_dir()?.join("fleet.json"))
}

/// The outcome of a load: the model to use, and — when the file was there but unusable —
/// the reason to show the operator, verbatim.
///
/// `failure` is `None` for a file that simply does not exist yet: that is the ordinary
/// first-run state, not a problem to report.
#[derive(Debug, Clone, PartialEq)]
pub struct FleetLoad {
    pub file: FleetFile,
    /// The parse/read error's own message, verbatim (FR-007). `Some` implies `file` is
    /// `FleetFile::default()` and the file on disk was left byte-for-byte untouched.
    pub failure: Option<String>,
}

/// Load the fleet model from `path`. An unreadable or invalid file yields in-memory
/// defaults, reports the verbatim reason, and leaves the file **byte-for-byte untouched** —
/// no rewrite, no `.corrupt` sidecar, nothing (FR-007, Principle VI). Unknown keys at a
/// known schema are ignored.
pub fn load_fleet_model_from(path: &Path) -> FleetLoad {
    if !path.exists() {
        return FleetLoad {
            file: FleetFile::default(),
            failure: None,
        };
    }
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            return FleetLoad {
                file: FleetFile::default(),
                failure: Some(e.to_string()),
            };
        }
    };
    match serde_json::from_str::<FleetFile>(&content) {
        Ok(file) => FleetLoad {
            file,
            failure: None,
        },
        Err(e) => {
            tracing::warn!(
                "{} is invalid; using defaults and leaving it untouched: {}",
                path.display(),
                e
            );
            FleetLoad {
                file: FleetFile::default(),
                failure: Some(e.to_string()),
            }
        }
    }
}

/// Load `fleet.json` from the configured state root.
pub fn load_fleet_model() -> FleetLoad {
    match fleet_file_path() {
        Ok(p) => load_fleet_model_from(&p),
        Err(e) => FleetLoad {
            file: FleetFile::default(),
            failure: Some(e.to_string()),
        },
    }
}

/// Atomically write the fleet model to `path` (temp file + rename). Writes `fleet.json` and
/// nothing else (FR-008).
pub fn save_fleet_model_to(path: &Path, file: &FleetFile) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    fs::write(&tmp, serde_json::to_string_pretty(file)?)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

/// Save `fleet.json` in full (it is small and entirely owned by this feature).
pub fn save_fleet_model(file: &FleetFile) -> Result<()> {
    let path = fleet_file_path()?;
    ensure_config_dir()?;
    save_fleet_model_to(&path, file)
}

#[cfg(test)]
mod fleet_file_tests {
    use super::*;
    use crate::protocol::{SettingOrigin, scan_fields_for_secrets};

    fn populated() -> FleetFile {
        FleetFile {
            schema: fleet_file_version(),
            members: BTreeMap::from([(
                "arn:aws:eks:eu-west-1:1234:cluster/prod-a".to_owned(),
                FleetMemberRecord {
                    display_name: Some("prod a".to_owned()),
                    labels: BTreeMap::from([
                        ("env".to_owned(), "prod".to_owned()),
                        ("region".to_owned(), "eu-west-1".to_owned()),
                    ]),
                    origins: BTreeMap::from([("display_name".to_owned(), SettingOrigin::Local)]),
                },
            )]),
            groups: BTreeMap::from([(
                "prod".to_owned(),
                FleetGroupRecord {
                    selector: "env in (prod,staging)".to_owned(),
                    origin: SettingOrigin::Local,
                },
            )]),
            adopted: BTreeMap::new(),
        }
    }

    #[test]
    fn roundtrip_and_unknown_keys_at_a_known_schema_are_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fleet.json");
        // Nothing on disk is the ordinary first-run state, not a failure to report.
        let fresh = load_fleet_model_from(&path);
        assert_eq!(fresh.file, FleetFile::default());
        assert_eq!(fresh.failure, None);
        assert_eq!(fresh.file.schema, 1);

        save_fleet_model_to(&path, &populated()).unwrap();
        let back = load_fleet_model_from(&path);
        assert_eq!(back.failure, None);
        assert_eq!(back.file, populated());

        // A future build's additive field is ignored, not a parse failure (FR-007).
        fs::write(
            &path,
            r#"{"schema":1,"members":{"a":{"labels":{"env":"prod"},"tier":"gold"}},"groups":{},"adopted":{},"fleetNickname":"team-a"}"#,
        )
        .unwrap();
        let forward = load_fleet_model_from(&path);
        assert_eq!(forward.failure, None, "unknown keys must be ignored");
        assert_eq!(
            forward.file.members["a"].labels["env"], "prod",
            "the keys we do know must still be read"
        );
    }

    /// T020 (SC-003, FR-007): "we fell back to defaults" and "we rewrote the operator's
    /// file" both look like success from outside. The bytes are what tell them apart, so
    /// this test reads them before and after and compares.
    #[test]
    fn invalid_file_yields_defaults_reports_the_reason_and_is_never_rewritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fleet.json");

        for broken in [
            // Truncated mid-object: valid JSON prefix, no closing brace.
            "{\n  \"schema\": 1,\n  \"members\": {\n    \"prod-a\": {\n      \"display_name\": \"prod a\"\n",
            // Not JSON at all.
            "this is not json, it is a note the operator left in the file\n",
        ] {
            fs::write(&path, broken).unwrap();
            let before = fs::read(&path).unwrap();

            let load = load_fleet_model_from(&path);
            assert_eq!(
                load.file,
                FleetFile::default(),
                "an invalid file must yield in-memory defaults"
            );
            let reason = load
                .failure
                .expect("an invalid file must report its own reason, not fail silently");
            // Verbatim: serde_json's own message, not a sentence we invented.
            let verbatim = serde_json::from_str::<FleetFile>(broken)
                .expect_err("the fixture must not parse")
                .to_string();
            assert_eq!(
                reason, verbatim,
                "the reason must be the parser's, verbatim"
            );

            let after = fs::read(&path).unwrap();
            assert_eq!(
                before, after,
                "fleet.json must be left byte-for-byte untouched: {broken:?}"
            );
            // Not rewritten *anywhere*: no sidecar either, so a later build that learns the
            // newer schema still has the operator's original bytes to read.
            let strays: Vec<_> = fs::read_dir(dir.path())
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .filter(|n| n != "fleet.json")
                .collect();
            assert!(strays.is_empty(), "nothing else may be written: {strays:?}");
        }
    }

    /// T021 (FR-020): the file holds context names, operator-authored strings and selector
    /// expressions. `scan_fields_for_secrets` is 014's own credential-shape detector
    /// (markers, bare tokens, JWTs, PEM blocks); every string we persist goes through it,
    /// and the serialized bytes are scanned as one field too so a secret smuggled in via a
    /// field this test forgot still trips it.
    #[test]
    fn fleet_file_holds_no_credential_shaped_value() {
        let file = populated();
        let json = serde_json::to_string_pretty(&file).unwrap();

        let mut fields: Vec<(String, String)> = Vec::new();
        for (context, member) in &file.members {
            fields.push(("members/<context>".to_owned(), context.clone()));
            if let Some(name) = &member.display_name {
                fields.push((format!("members/{context}/display_name"), name.clone()));
            }
            for (k, v) in &member.labels {
                fields.push((format!("members/{context}/labels/<key>"), k.clone()));
                fields.push((format!("members/{context}/labels/{k}"), v.clone()));
            }
        }
        for (name, group) in &file.groups {
            fields.push(("groups/<name>".to_owned(), name.clone()));
            fields.push((format!("groups/{name}/selector"), group.selector.clone()));
        }
        fields.push(("fleet.json".to_owned(), json.clone()));

        let borrowed: Vec<(&str, &str)> = fields
            .iter()
            .map(|(n, v)| (n.as_str(), v.as_str()))
            .collect();
        assert_eq!(
            scan_fields_for_secrets(&borrowed),
            Ok(()),
            "fleet.json must hold no credential-shaped value"
        );

        // And the shape itself has nowhere to put one: these are the only keys.
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        let mut keys: Vec<&str> = parsed
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(keys, ["adopted", "groups", "members", "schema"]);
    }

    /// T022 (FR-008): saving the fleet model writes `fleet.json` and nothing else. The two
    /// neighbours most at risk of being co-written are compared byte-for-byte.
    #[test]
    fn saving_the_fleet_model_touches_no_other_file() {
        let dir = tempfile::tempdir().unwrap();
        let cluster_settings = dir.path().join("cluster-settings.json");
        let federation = dir.path().join("federation.json");
        fs::write(&cluster_settings, r#"{"prod-a":{"schema":1}}"#).unwrap();
        fs::write(
            &federation,
            r#"{"version":1,"contexts":["prod-a"],"softCap":8}"#,
        )
        .unwrap();
        let settings_before = fs::read(&cluster_settings).unwrap();
        let federation_before = fs::read(&federation).unwrap();

        save_fleet_model_to(&dir.path().join("fleet.json"), &populated()).unwrap();

        assert_eq!(
            fs::read(&cluster_settings).unwrap(),
            settings_before,
            "cluster-settings.json must be unmodified"
        );
        assert_eq!(
            fs::read(&federation).unwrap(),
            federation_before,
            "federation.json must be unmodified"
        );
        let mut written: Vec<String> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        written.sort();
        assert_eq!(
            written,
            ["cluster-settings.json", "federation.json", "fleet.json"],
            "only fleet.json may appear"
        );
    }
}
