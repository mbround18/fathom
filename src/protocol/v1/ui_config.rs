//! Persisted UI preferences (`~/.fathom/config.json → ui`).
//!
//! See `specs/001-tauri-shadcn-frontend/data-model.md` §2. The blob is owned by the UI
//! client but stored by the backend; `navigation.expanded_groups` and
//! `auto_start.mcp_stdio` stay at their existing keys and travel alongside it in
//! `BackendCmd::SaveUiConfig` / `SystemEvent::UiConfigLoaded`.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use ts_rs::TS;

use super::{Route, SectionKey};

/// The only `ui.schema` version this build understands.
pub const UI_CONFIG_SCHEMA: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct UiConfig {
    /// Always `1`. Any other value is treated as invalid on load.
    #[ts(type = "1")]
    pub schema: u32,
    #[serde(default)]
    pub theme: ThemePrefs,
    #[serde(default)]
    pub density: DensityPrefs,
    #[serde(default)]
    pub layouts: BTreeMap<WidthClass, LayoutProfile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_selection: Option<LastSelection>,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            schema: UI_CONFIG_SCHEMA,
            theme: ThemePrefs::default(),
            density: DensityPrefs::default(),
            layouts: BTreeMap::new(),
            last_selection: None,
        }
    }
}

impl UiConfig {
    /// Semantic checks that serde's shape validation can't express.
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != UI_CONFIG_SCHEMA {
            return Err(format!(
                "unsupported ui.schema {} (expected {})",
                self.schema, UI_CONFIG_SCHEMA
            ));
        }
        for (class, profile) in &self.layouts {
            for (i, slot) in profile.slots.iter().enumerate() {
                if !is_pct(slot.size_pct) {
                    return Err(format!(
                        "layouts.{class:?}.slots[{i}].sizePct out of range: {}",
                        slot.size_pct
                    ));
                }
            }
            if let Some(dock) = &profile.dock
                && !is_pct(dock.height_pct)
            {
                return Err(format!(
                    "layouts.{class:?}.dock.heightPct out of range: {}",
                    dock.height_pct
                ));
            }
        }
        Ok(())
    }

    /// Parse and validate a raw `config.json → ui` value.
    pub fn from_value(value: serde_json::Value) -> Result<Self, String> {
        let ui: UiConfig = serde_json::from_value(value).map_err(|e| e.to_string())?;
        ui.validate()?;
        Ok(ui)
    }
}

fn is_pct(v: f64) -> bool {
    v.is_finite() && (0.0..=100.0).contains(&v)
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ThemePrefs {
    #[serde(default)]
    pub mode: ThemeMode,
    #[serde(default)]
    pub accent: ThemeAccent,
    #[serde(default)]
    pub effects: ThemeEffects,
    #[serde(default)]
    pub glitch: bool,
}

/// `dark` = neon-noir (default), `light` = blueprint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    #[default]
    Dark,
    Light,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
pub enum ThemeAccent {
    #[default]
    Cyan,
    Magenta,
}

/// Scanlines, glow, and grain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
pub enum ThemeEffects {
    #[default]
    On,
    Off,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DensityPrefs {
    /// Beats the width class's default density when set.
    #[serde(rename = "override", default, skip_serializing_if = "Option::is_none")]
    pub override_density: Option<Density>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
pub enum Density {
    Comfortable,
    Compact,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
pub enum WidthClass {
    Phone,
    Tablet,
    Laptop,
    Desktop,
    Wide,
    Ultra,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LayoutProfile {
    /// Left to right.
    pub slots: Vec<SlotConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dock: Option<DockConfig>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SlotConfig {
    pub kind: SlotKind,
    /// From react-resizable-panels (0–100).
    pub size_pct: f64,
    /// List slots only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pinned_route: Option<Route>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<ToolKind>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
pub enum SlotKind {
    Nav,
    List,
    Inspector,
    Tool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DockConfig {
    pub open: bool,
    /// 0–100.
    pub height_pct: f64,
    pub tabs: Vec<ToolKind>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
pub enum ToolKind {
    Overview,
    Metrics,
    Flux,
    /// 028-knative-pane: Knative Serving tool pane.
    Knative,
    Hubble,
    Logs,
    Events,
    Portforwards,
    /// 007-topology-graph-view: the cluster topology graph tool pane.
    Topology,
    /// 022-velero-backup-pane: read-only Velero backup health tool pane.
    Velero,
    /// 023-cert-manager-certificates-pane: the Certificates (cert-manager) tool pane.
    #[serde(rename = "certManager")]
    CertManager,
    /// 021-trivy-vulnerability-pane: the cluster-wide Trivy Operator vulnerability summary.
    Vulnerabilities,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LastSelection {
    pub context: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    pub section: SectionKey,
}
