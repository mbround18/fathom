//! MCP (Model Context Protocol) server for Fathom
//! Exposes application state, logs, and Kubernetes information via MCP tools.
//! HTTP server on port 18288 (1.8288m ≈ 1 fathom)
//!
//! MCP allows Claude and other AI assistants to query Fathom's state while it's running,
//! enabling real-time debugging, status checks, and integration with other tools.

use anyhow::Result;
use axum::{
    Router,
    extract::State as AxumState,
    response::{IntoResponse, Json},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tower_http::cors::CorsLayer;

/// Shared application state accessible to MCP server
/// Captures logs, errors, and status messages from Fathom
#[derive(Clone, Debug)]
pub struct McpAppState {
    /// Recent status/info messages (max 1000)
    pub status_messages: Arc<RwLock<Vec<String>>>,
    /// Recent error messages (max 500)
    pub error_messages: Arc<RwLock<Vec<String>>>,
    /// Current cluster context
    pub current_context: Arc<RwLock<Option<String>>>,
    /// List of available namespaces
    pub namespaces: Arc<RwLock<Vec<String>>>,
    /// MCP server is running
    pub is_running: Arc<RwLock<bool>>,
}

impl McpAppState {
    pub fn new() -> Self {
        Self {
            status_messages: Arc::new(RwLock::new(Vec::new())),
            error_messages: Arc::new(RwLock::new(Vec::new())),
            current_context: Arc::new(RwLock::new(None)),
            namespaces: Arc::new(RwLock::new(Vec::new())),
            is_running: Arc::new(RwLock::new(false)),
        }
    }

    /// Check if MCP server is running
    pub async fn is_running_status(&self) -> bool {
        *self.is_running.read().await
    }

    /// Set MCP server running status
    pub async fn set_running(&self, running: bool) {
        *self.is_running.write().await = running;
    }

    /// Add a status message to the log (auto-maintains buffer)
    pub async fn push_status(&self, msg: String) {
        let mut messages = self.status_messages.write().await;
        messages.push(msg);
        // Keep last 1000 messages
        if messages.len() > 1000 {
            let drain_to = messages.len() - 1000;
            messages.drain(0..drain_to);
        }
    }

    /// Add an error message to the log (auto-maintains buffer)
    pub async fn push_error(&self, msg: String) {
        let mut messages = self.error_messages.write().await;
        messages.push(msg);
        // Keep last 500 messages
        if messages.len() > 500 {
            let drain_to = messages.len() - 500;
            messages.drain(0..drain_to);
        }
    }

    /// Update current cluster context
    pub async fn set_context(&self, context: String) {
        *self.current_context.write().await = Some(context);
    }

    /// Update available namespaces
    pub async fn set_namespaces(&self, namespaces: Vec<String>) {
        *self.namespaces.write().await = namespaces;
    }

    /// Get status snapshot for query
    pub async fn get_status(&self, limit: usize) -> Vec<String> {
        let messages = self.status_messages.read().await;
        let start = if messages.len() > limit {
            messages.len() - limit
        } else {
            0
        };
        messages[start..].to_vec()
    }

    /// Get error logs snapshot for query
    pub async fn get_errors(&self, limit: usize) -> Vec<String> {
        let messages = self.error_messages.read().await;
        let start = if messages.len() > limit {
            messages.len() - limit
        } else {
            0
        };
        messages[start..].to_vec()
    }

    /// Get all logs (status + errors) with optional limit
    pub async fn get_all_logs(&self, limit: usize) -> serde_json::Value {
        let status = self.get_status(limit).await;
        let errors = self.get_errors(limit / 2).await;
        let context = self.current_context.read().await;
        let namespaces = self.namespaces.read().await;

        json!({
            "status_messages": status,
            "error_messages": errors,
            "current_context": context.clone(),
            "namespaces": namespaces.clone(),
        })
    }
}

impl Default for McpAppState {
    fn default() -> Self {
        Self::new()
    }
}

/// Get the MCP socket directory: ~/.fathom/mcp/
pub fn mcp_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Could not find home directory"))?;
    let path = home.join(".fathom").join("mcp");
    Ok(path)
}

/// Get the MCP socket path: ~/.fathom/mcp/socket
pub fn mcp_socket_path() -> Result<PathBuf> {
    let dir = mcp_dir()?;
    Ok(dir.join("socket"))
}

/// Get the MCP generic config file path: ~/.fathom/mcp/generic.json
pub fn mcp_generic_config_path() -> Result<PathBuf> {
    let dir = mcp_dir()?;
    Ok(dir.join("generic.json"))
}

/// Get the MCP VSCode config file path: ~/.fathom/mcp/vscode.json
pub fn mcp_vscode_config_path() -> Result<PathBuf> {
    let dir = mcp_dir()?;
    Ok(dir.join("vscode.json"))
}

/// Initialize MCP server socket and configuration files
pub async fn initialize_mcp_server(state: McpAppState) -> Result<()> {
    let dir = mcp_dir()?;

    // Create parent directory if needed
    tokio::fs::create_dir_all(&dir).await?;

    // Create socket file (placeholder, will be real socket when transport added)
    let socket_path = mcp_socket_path()?;
    if !socket_path.exists() {
        tokio::fs::write(&socket_path, "").await?;
    }

    // Generate generic.json for any MCP client
    let generic_config = create_generic_config()?;
    let generic_path = mcp_generic_config_path()?;
    tokio::fs::write(
        &generic_path,
        serde_json::to_string_pretty(&generic_config)?,
    )
    .await?;

    // Generate vscode.json with VSCode-specific settings
    let vscode_config = create_vscode_config()?;
    let vscode_path = mcp_vscode_config_path()?;
    tokio::fs::write(&vscode_path, serde_json::to_string_pretty(&vscode_config)?).await?;

    state.set_running(true).await;

    tracing::info!("MCP server initialized at {:?}", dir);
    tracing::info!("Generated config files:");
    tracing::info!("  - generic.json: {:?}", generic_path);
    tracing::info!("  - vscode.json: {:?}", vscode_path);
    tracing::info!("HTTP MCP server available at: http://127.0.0.1:18288/mcp");
    tracing::info!("Available MCP tools:");
    tracing::info!("  - get_status: Get recent status messages");
    tracing::info!("  - get_logs: Get error and status logs");
    tracing::info!("  - get_app_state: Get current app state (context, namespaces)");
    tracing::info!("  - list_namespaces: List cluster namespaces");
    tracing::info!("  - list_deployments: List deployments");
    tracing::info!("  - restart_deployment: Restart a deployment (requires confirmation)");
    tracing::info!("  - get_pod_logs: Fetch pod logs");
    tracing::info!("  - health_check: Verify Fathom is running");

    Ok(())
}

/// Create generic MCP configuration
fn create_generic_config() -> Result<serde_json::Value> {
    Ok(json!({
        "name": "fathom",
        "version": "0.1.0",
        "description": "Kubernetes dashboard with MCP integration for AI debugging",
        "transport": "http",
        "url": "http://127.0.0.1:18288/mcp",
        "port": 18288,
        "note": "Port 18288: 1.8288 meters ≈ 1 fathom (unit of depth measurement)",
        "tools": [
            {
                "name": "get_status",
                "description": "Get recent application status messages from Fathom",
                "parameters": {
                    "limit": {"type": "integer", "default": 50},
                    "pattern": {"type": "string"}
                }
            },
            {
                "name": "get_logs",
                "description": "Get recent logs from Fathom (status and errors)",
                "parameters": {
                    "log_type": {"type": "string", "enum": ["all", "status", "errors"]},
                    "limit": {"type": "integer", "default": 100}
                }
            },
            {
                "name": "get_app_state",
                "description": "Get current application state (context, namespaces)",
                "parameters": {}
            },
            {
                "name": "health_check",
                "description": "Check if Fathom is running and responsive",
                "parameters": {}
            },
            {
                "name": "list_namespaces",
                "description": "List namespaces in the current cluster",
                "parameters": {
                    "limit": {"type": "integer", "default": 200}
                }
            },
            {
                "name": "list_deployments",
                "description": "List deployments (optionally scoped to a namespace)",
                "parameters": {
                    "namespace": {"type": "string"},
                    "limit": {"type": "integer", "default": 200}
                }
            },
            {
                "name": "restart_deployment",
                "description": "Restart a deployment with optional confirmation",
                "parameters": {
                    "namespace": {"type": "string"},
                    "name": {"type": "string"},
                    "confirm": {"type": "boolean", "default": true}
                }
            },
            {
                "name": "get_pod_logs",
                "description": "Fetch recent pod logs (optionally pick container)",
                "parameters": {
                    "namespace": {"type": "string"},
                    "pod": {"type": "string"},
                    "container": {"type": "string"},
                    "tail_lines": {"type": "integer", "default": 200}
                }
            }
        ],
        "capabilities": {
            "tools": true,
            "resources": true,
            "sampling": false
        },
        "setup_instructions": {
            "generic_mcp_client": "Configure your MCP client to connect to http://127.0.0.1:18288/mcp",
            "claude_integration": "1. Visit claude.anthropic.com\\n2. Go to Settings → Extensions\\n3. Add MCP server with HTTP endpoint http://127.0.0.1:18288/mcp\\n4. Select tools you want Claude to use",
            "launch_command": "fathom --mcp-http   # Start Fathom with MCP HTTP server\\nfathom --only-mcp  # Run MCP server only (headless)"
        }
    }))
}

/// Create VSCode-specific MCP configuration (mcp.json format)
fn create_vscode_config() -> Result<serde_json::Value> {
    Ok(json!({
        "mcpServers": {
            "fathom": {
                "type": "http",
                "url": "http://127.0.0.1:18288/mcp",
                "command": "fathom",
                "args": ["--mcp-http"],
                "env": {
                    "FATHOM_MCP_HTTP_ENABLED": "true",
                    "FATHOM_MCP_PORT": "18288"
                }
            }
        },
        "_comment": "This file follows VSCode's mcp.json format. Copy the 'fathom' server config to your .vscode/mcp.json file.",
        "_note": "Port 18288: 1.8288 meters ≈ 1 fathom (unit of depth measurement)",
        "_setup_instructions": {
            "step_1": "Ensure you have VSCode 1.102+ and GitHub Copilot enabled",
            "step_2": "Run the command: MCP: Open User Configuration (or MCP: Open Workspace Folder Configuration)",
            "step_3": "This will open your mcp.json file",
            "step_4": "Copy the 'fathom' server configuration from above into the 'mcpServers' section",
            "step_5": "Save the file - the HTTP server will start automatically when you launch Fathom",
            "step_6": "Open Copilot Chat and use # to access Fathom tools",
            "docs_url": "https://code.visualstudio.com/docs/copilot/customization/mcp-servers",
            "launch_options": {
                "gui_with_mcp": "fathom --mcp-http",
                "headless_mcp_only": "fathom --only-mcp",
                "custom_port": "fathom --mcp-http --mcp-port 8080"
            }
        },
        "_description": "Fathom MCP Server - Kubernetes dashboard with AI integration",
        "_available_tools": [
            {
                "name": "get_status",
                "description": "Get recent application status messages from Fathom"
            },
            {
                "name": "get_logs",
                "description": "Get recent logs from Fathom (status and errors)"
            },
            {
                "name": "get_app_state",
                "description": "Get current application state (context, namespaces)"
            },
            {
                "name": "health_check",
                "description": "Check if Fathom is running and responsive"
            },
            {
                "name": "list_namespaces",
                "description": "List namespaces in the current cluster"
            },
            {
                "name": "list_deployments",
                "description": "List deployments (optionally scoped to a namespace)"
            },
            {
                "name": "restart_deployment",
                "description": "Restart a deployment with optional confirmation"
            },
            {
                "name": "get_pod_logs",
                "description": "Fetch recent pod logs (optionally pick container)"
            }
        ]
    }))
}

/// Shutdown MCP server
pub async fn shutdown_mcp_server(state: McpAppState) -> Result<()> {
    state.set_running(false).await;
    tracing::info!("MCP server shutdown");
    Ok(())
}

// ============================================================================
// HTTP MCP Server Implementation
// ============================================================================

#[derive(Deserialize)]
struct McpRequest {
    jsonrpc: Option<String>,
    id: Option<serde_json::Value>,
    method: String,
    params: Option<serde_json::Value>,
}

#[derive(Serialize)]
struct McpResponse {
    jsonrpc: String,
    id: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct ToolCallRequest {
    tool: String,
    #[serde(default)]
    arguments: serde_json::Value,
}

#[derive(Serialize)]
struct ToolListResponse {
    tools: Vec<ToolDefinition>,
}

#[derive(Serialize)]
struct ToolDefinition {
    name: String,
    description: String,
}

/// HTTP handler: POST /mcp - Handle MCP JSON-RPC protocol
async fn handle_mcp_call(
    AxumState(state): AxumState<Arc<McpAppState>>,
    Json(req): Json<McpRequest>,
) -> impl IntoResponse {
    let id = req.id.clone();
    let jsonrpc = req.jsonrpc.unwrap_or_else(|| "2.0".to_string());

    let result = match req.method.as_str() {
        "tools/list" => {
            let tools = vec![
                ToolDefinition {
                    name: "get_status".to_string(),
                    description: "Get recent application status messages from Fathom".to_string(),
                },
                ToolDefinition {
                    name: "get_logs".to_string(),
                    description: "Get recent logs from Fathom (status and errors)".to_string(),
                },
                ToolDefinition {
                    name: "get_app_state".to_string(),
                    description: "Get current application state (context, namespaces)".to_string(),
                },
                ToolDefinition {
                    name: "health_check".to_string(),
                    description: "Check if Fathom is running and responsive".to_string(),
                },
                ToolDefinition {
                    name: "list_namespaces".to_string(),
                    description: "List namespaces in the current cluster".to_string(),
                },
                ToolDefinition {
                    name: "list_deployments".to_string(),
                    description: "List deployments (optionally scoped to a namespace)".to_string(),
                },
                ToolDefinition {
                    name: "restart_deployment".to_string(),
                    description: "Restart a deployment with optional confirmation".to_string(),
                },
                ToolDefinition {
                    name: "get_pod_logs".to_string(),
                    description: "Fetch recent pod logs (optionally pick container)".to_string(),
                },
            ];
            Ok(json!({ "tools": tools }))
        }
        "tools/call" => {
            if let Some(params) = req.params {
                handle_tool_call_impl(state, params).await
            } else {
                Err(json!({
                    "code": -32602,
                    "message": "Missing params"
                }))
            }
        }
        "initialize" => Ok(json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {
                "tools": {}
            },
            "serverInfo": {
                "name": "fathom",
                "version": "0.1.0"
            }
        })),
        _ => Err(json!({
            "code": -32601,
            "message": format!("Unknown method: {}", req.method)
        })),
    };

    let response = McpResponse {
        jsonrpc,
        id,
        result: result.as_ref().ok().cloned(),
        error: result.as_ref().err().cloned(),
    };

    Json(response)
}

/// Handle tool invocation (extracted for reuse)
async fn handle_tool_call_impl(
    state: Arc<McpAppState>,
    params: serde_json::Value,
) -> Result<serde_json::Value, serde_json::Value> {
    let tool_name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
    let arguments = params.get("arguments").cloned().unwrap_or(json!({}));

    match tool_name {
        "health_check" => Ok(json!({
            "status": "ok",
            "message": "Fathom is running"
        })),
        "get_status" => {
            let limit = arguments
                .get("limit")
                .and_then(|v| v.as_u64())
                .unwrap_or(50) as usize;
            let messages = state.get_status(limit).await;
            Ok(json!({
                "messages": messages
            }))
        }
        "get_logs" => {
            let limit = arguments
                .get("limit")
                .and_then(|v| v.as_u64())
                .unwrap_or(100) as usize;
            let logs = state.get_all_logs(limit).await;
            Ok(logs)
        }
        "get_app_state" => {
            let context = state.current_context.read().await;
            let namespaces = state.namespaces.read().await;
            Ok(json!({
                "current_context": context.clone(),
                "namespaces": namespaces.clone()
            }))
        }
        "list_namespaces" => {
            let namespaces = state.namespaces.read().await;
            Ok(json!({
                "namespaces": namespaces.clone()
            }))
        }
        "list_deployments" | "restart_deployment" | "get_pod_logs" => {
            // TODO: Wire to backend via command channel
            Err(json!({
                "code": -32700,
                "message": format!("Tool '{}' not yet fully implemented", tool_name)
            }))
        }
        _ => Err(json!({
            "code": -32601,
            "message": format!("Unknown tool: {}", tool_name)
        })),
    }
}

/// HTTP handler: GET /mcp/health (for direct health checks)
async fn handle_health() -> impl IntoResponse {
    Json(json!({
        "status": "ok",
        "service": "fathom-mcp",
        "version": "0.1.0"
    }))
}

/// HTTP handler: GET /mcp/state (for direct state queries)
async fn handle_get_state(AxumState(state): AxumState<Arc<McpAppState>>) -> impl IntoResponse {
    let context = state.current_context.read().await;
    let namespaces = state.namespaces.read().await;

    Json(json!({
        "current_context": context.clone(),
        "namespaces": namespaces.clone()
    }))
}

/// HTTP handler: GET /mcp/tools (for direct tool listing)
async fn handle_list_tools() -> impl IntoResponse {
    let tools = vec![
        ToolDefinition {
            name: "get_status".to_string(),
            description: "Get recent application status messages from Fathom".to_string(),
        },
        ToolDefinition {
            name: "get_logs".to_string(),
            description: "Get recent logs from Fathom (status and errors)".to_string(),
        },
        ToolDefinition {
            name: "get_app_state".to_string(),
            description: "Get current application state (context, namespaces)".to_string(),
        },
        ToolDefinition {
            name: "health_check".to_string(),
            description: "Check if Fathom is running and responsive".to_string(),
        },
        ToolDefinition {
            name: "list_namespaces".to_string(),
            description: "List namespaces in the current cluster".to_string(),
        },
        ToolDefinition {
            name: "list_deployments".to_string(),
            description: "List deployments (optionally scoped to a namespace)".to_string(),
        },
        ToolDefinition {
            name: "restart_deployment".to_string(),
            description: "Restart a deployment with optional confirmation".to_string(),
        },
        ToolDefinition {
            name: "get_pod_logs".to_string(),
            description: "Fetch recent pod logs (optionally pick container)".to_string(),
        },
    ];

    Json(ToolListResponse { tools })
}

pub async fn run_http_mcp_server(state: Arc<McpAppState>, port: u16) -> Result<()> {
    state.set_running(true).await;

    let app = Router::new()
        .route("/mcp", post(handle_mcp_call))
        .route("/mcp/health", get(handle_health))
        .route("/mcp/state", get(handle_get_state))
        .route("/mcp/tools", get(handle_list_tools))
        .layer(CorsLayer::permissive())
        .with_state(state.clone());

    let addr = format!("127.0.0.1:{}", port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;

    tracing::info!("MCP HTTP server listening on http://{}", addr);
    tracing::info!("Available endpoints:");
    tracing::info!("  POST /mcp              - MCP JSON-RPC protocol handler");
    tracing::info!("  GET  /mcp/health       - Health check");
    tracing::info!("  GET  /mcp/state        - Current app state");
    tracing::info!("  GET  /mcp/tools        - List available tools");

    axum::serve(listener, app).await?;

    state.set_running(false).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mcp_dir() {
        let result = mcp_dir();
        assert!(result.is_ok());
        let path = result.unwrap();
        assert!(path.to_string_lossy().contains(".fathom"));
        assert!(path.to_string_lossy().contains("mcp"));
    }

    #[test]
    fn test_mcp_socket_path() {
        let result = mcp_socket_path();
        assert!(result.is_ok());
        let path = result.unwrap();
        assert!(path.to_string_lossy().contains("socket"));
    }

    #[tokio::test]
    async fn test_app_state() {
        let state = McpAppState::new();
        state.push_status("test message".to_string()).await;
        state.set_context("test-context".to_string()).await;
        state.set_namespaces(vec!["default".to_string()]).await;

        let messages = state.status_messages.read().await;
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0], "test message");

        let ctx = state.current_context.read().await;
        assert_eq!(ctx.as_ref(), Some(&"test-context".to_string()));

        let ns = state.namespaces.read().await;
        assert_eq!(ns.len(), 1);
    }

    #[tokio::test]
    async fn test_message_buffer_limit() {
        let state = McpAppState::new();

        // Add 1500 messages (more than limit of 1000)
        for i in 0..1500 {
            state.push_status(format!("message {}", i)).await;
        }

        let messages = state.status_messages.read().await;
        assert_eq!(messages.len(), 1000, "Should maintain max 1000 messages");
        // Should contain last 1000 (500-1499)
        assert_eq!(messages[0], "message 500");
        assert_eq!(messages[999], "message 1499");
    }

    #[tokio::test]
    async fn test_get_status() {
        let state = McpAppState::new();
        for i in 0..100 {
            state.push_status(format!("msg {}", i)).await;
        }

        let status = state.get_status(10).await;
        assert_eq!(status.len(), 10);
        assert_eq!(status[0], "msg 90");
        assert_eq!(status[9], "msg 99");
    }
}
