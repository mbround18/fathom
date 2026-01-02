use super::traits::{ViewAction, ViewComponent};
use crate::protocol::{BackendCmd, BackendEvent};
use eframe::egui;

/// MCP (Model Context Protocol) Server integration view
pub struct McpIntegrationView {
    /// Server is currently running
    mcp_running: bool,
    /// Auto-start MCP server on launch
    auto_start: bool,
    /// Status message
    status_message: Option<String>,
    /// Track if preferences have changed
    preferences_changed: bool,
}

impl McpIntegrationView {
    pub fn new() -> Self {
        Self {
            mcp_running: false,
            auto_start: false,
            status_message: None,
            preferences_changed: false,
        }
    }
}

impl ViewComponent for McpIntegrationView {
    fn id(&self) -> &'static str {
        "mcp_integration"
    }

    fn render(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) -> Vec<ViewAction> {
        let mut actions = Vec::new();

        ui.heading("MCP (Model Context Protocol) Server");
        ui.separator();

        // Status section
        ui.label("Status:");
        let status_text = if self.mcp_running {
            "🟢 Running"
        } else {
            "🔴 Stopped"
        };
        ui.label(egui::RichText::new(status_text).color(if self.mcp_running {
            egui::Color32::GREEN
        } else {
            egui::Color32::RED
        }));

        ui.add_space(12.0);

        // Control buttons
        ui.horizontal(|ui| {
            if self.mcp_running {
                if ui.button("⏹ Stop MCP Server").clicked() {
                    actions.push(ViewAction::SendCommand(BackendCmd::StopMcpServer));
                    self.status_message = Some("Stopping MCP server...".to_string());
                }
            } else {
                if ui.button("▶ Start MCP Server").clicked() {
                    actions.push(ViewAction::SendCommand(BackendCmd::StartMcpServer));
                    self.status_message = Some("Starting MCP server...".to_string());
                }
            }
        });

        ui.add_space(8.0);

        // Auto-start toggle
        let previous_auto_start = self.auto_start;
        ui.horizontal(|ui| {
            ui.label("Auto-start on launch:");
            ui.checkbox(&mut self.auto_start, "Enabled");
        });

        // If auto_start preference changed, save it
        if self.auto_start != previous_auto_start {
            self.preferences_changed = true;
            let _ = crate::backend::persistence::save_mcp_auto_start(self.auto_start);
        }

        ui.add_space(12.0);
        ui.separator();
        ui.add_space(8.0);

        // Status message
        if let Some(msg) = &self.status_message {
            ui.label(egui::RichText::new(msg).color(egui::Color32::YELLOW));
            ui.add_space(8.0);
        }

        // Information section
        ui.group(|ui| {
            ui.label(egui::RichText::new("Configuration Files:").strong());
            ui.add_space(4.0);

            ui.label("MCP config files are generated at:");
            ui.code("~/.fathom/mcp/");
            ui.add_space(8.0);

            ui.label("• generic.json - For any MCP-compatible client");
            ui.label("• vscode.json - Specific settings for VSCode");
            ui.label("• socket - MCP communication endpoint");
        });

        ui.add_space(12.0);

        // Integration instructions
        ui.group(|ui| {
            ui.label(egui::RichText::new("Integration Instructions:").strong());
            ui.add_space(8.0);

            ui.collapsing("VSCode Copilot Integration (Recommended)", |ui| {
                ui.add_space(4.0);
                ui.label("Native VSCode Copilot MCP support (VSCode 1.102+):");
                ui.add_space(4.0);
                ui.label("1. Open Command Palette (Ctrl+Shift+P)");
                ui.label("2. Run: MCP: Open User Configuration");
                ui.label("3. Copy the Fathom server config from vscode.json");
                ui.label("4. Paste into your mcp.json under 'mcpServers'");
                ui.label("5. Save - VSCode auto-starts the server");
                ui.label("6. In Copilot Chat, use # to access tools");
                ui.add_space(4.0);
                ui.label("📄 Config file: ~/.fathom/mcp/vscode.json");
                ui.hyperlink_to(
                    "📖 Official VSCode MCP docs",
                    "https://code.visualstudio.com/docs/copilot/customization/mcp-servers",
                );
            });

            ui.collapsing("Claude Integration", |ui| {
                ui.add_space(4.0);
                ui.label("1. Visit https://claude.anthropic.com");
                ui.label("2. Go to Settings → Extensions");
                ui.label("3. Add MCP server");
                ui.label("4. Use settings from generic.json");
                ui.label("5. Select tools you want Claude to access");
                ui.add_space(4.0);
                ui.label("See generic.json in ~/.fathom/mcp/ for details");
            });

            ui.collapsing("Generic MCP Client", |ui| {
                ui.add_space(4.0);
                ui.label("Configure your MCP client with:");
                ui.code("~/.fathom/mcp/generic.json");
                ui.add_space(4.0);
                ui.label("Supported transport: stdio");
                ui.label("Tools: get_status, get_logs, get_app_state, health_check");
            });
        });

        ui.add_space(12.0);

        // Features section
        ui.group(|ui| {
            ui.label(egui::RichText::new("Available Tools:").strong());
            ui.add_space(8.0);

            ui.label("• get_status - Get recent Fathom status messages");
            ui.label("• get_logs - Retrieve application error and status logs");
            ui.label("• get_app_state - Get current cluster context and namespaces");
            ui.label("• health_check - Verify Fathom is running");
        });

        ui.add_space(12.0);

        // Config file viewer
        if ui.button("📋 View Configuration Files").clicked() {
            actions.push(ViewAction::SendCommand(BackendCmd::OpenMcpConfigDir));
            self.status_message = Some("Opening MCP config directory...".to_string());
        }

        actions
    }

    fn on_event(&mut self, event: &BackendEvent) {
        match event {
            BackendEvent::System(crate::protocol::SystemEvent::McpServerStatus {
                running,
                message,
            }) => {
                self.mcp_running = *running;
                self.status_message = Some(message.clone());
            }
            _ => {}
        }
    }

    fn on_mount(&mut self) -> Vec<ViewAction> {
        // Load auto-start preference from config
        if let Ok(auto_start) = crate::backend::persistence::load_mcp_auto_start() {
            self.auto_start = auto_start;
        }
        Vec::new()
    }

    fn on_unmount(&mut self) -> Vec<ViewAction> {
        Vec::new()
    }

    fn render_details(
        &mut self,
        _ui: &mut egui::Ui,
        _ctx: &egui::Context,
    ) -> (bool, Vec<ViewAction>) {
        (false, Vec::new())
    }
}
