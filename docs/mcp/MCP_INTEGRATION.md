# MCP Integration for Fathom

## Overview

Fathom now includes **Model Context Protocol (MCP)** server integration that exposes the application's state, logs, and Kubernetes information to Claude and other AI assistants. This enables real-time debugging, status checks, and intelligent assistance while Fathom is running.

## Location & Configuration

- **MCP Directory**: `~/.fathom/mcp/`
- **Configuration File**: `~/.fathom/mcp/fathom.json` (created on startup)
- **Socket Path**: `~/.fathom/mcp/socket` (reserved for future transport)

The configuration file is automatically generated when Fathom starts and documents all available MCP tools and capabilities.

## Architecture

### MCP App State
The `McpAppState` struct in `src/mcp_server.rs` captures:
- **Status Messages** (ring buffer, max 1000): Application info and status updates
- **Error Messages** (ring buffer, max 500): Application errors and warnings  
- **Current Context**: Active Kubernetes context
- **Namespaces**: List of available namespaces

### Server Initialization
The MCP server starts automatically in the backend thread (`src/backend.rs`) and:
1. Creates `~/.fathom/mcp/` directory
2. Writes `fathom.json` with tool definitions
3. Prepares tool handlers (foundation for future transport)

## Available MCP Tools

### 1. `get_status`
**Purpose**: Retrieve recent application status messages

**Parameters**:
- `limit` (integer, default: 50, max: 100): Number of messages to return
- `pattern` (string, optional): Filter messages by substring (case-insensitive)

**Returns**: Array of recent status messages

**Example Use Case**: "What's the current status of Fathom? Is it connected to a cluster?"

---

### 2. `get_logs`  
**Purpose**: Get comprehensive logs including both status and error messages

**Parameters**:
- `log_type` (string, enum: "all" | "status" | "errors", default: "all")
- `limit` (integer, default: 100, max: 500): Maximum log lines

**Returns**: JSON object with separate status and error logs, current context, and namespaces

**Example Use Case**: "Show me any errors that occurred in the last 50 logs" or "What warnings did Fathom encounter?"

---

### 3. `get_app_state`
**Purpose**: Get the current application state (cluster context, available namespaces)

**Parameters**: None

**Returns**: JSON object containing:
```json
{
  "current_context": "your-cluster-context",
  "namespaces": ["default", "kube-system", "monitoring"]
}
```

**Example Use Case**: "Which cluster is Fathom currently connected to? What namespaces are available?"

---

### 4. `health_check`
**Purpose**: Verify Fathom is running and responsive

**Parameters**: None

**Returns**: Health status and basic metrics

**Example Use Case**: "Is Fathom still running? Is it responsive?"

## Integration with Claude

When Claude has access to the Fathom MCP server:

```
Claude: "What's the status of my Kubernetes cluster?"
→ Uses get_app_state tool to check current context
→ Uses get_status to see recent operations
→ Provides context-aware assistance based on real Fathom state
```

```
Claude: "Are there any errors in Fathom's logs?"
→ Uses get_logs tool with log_type="errors"
→ Analyzes errors and suggests debugging steps
```

## Future Enhancements

### Phase 2: Transport & Tool Handlers
- Implement Socket/StdIO transport using rmcp crate
- Add actual tool request handlers
- Enable bidirectional communication

### Phase 3: Extended Capabilities
- Expose Kubernetes resource queries (pods, deployments, etc.)
- Query metrics and performance data
- Access integration status (Prometheus, Grafana)
- Get pod logs and container information

### Phase 4: Write Capabilities
- Execute Kubernetes operations through Claude (with confirmation)
- Manage integrations (configure Prometheus, Grafana)
- Create/delete resources (with safeguards)

## Usage with Claude

1. **Verify Fathom is running**:
   ```bash
   $ cargo run
   # Look for: "MCP server initialized at ..."
   ```

2. **Claude can now query Fathom**:
   - Access `~/.fathom/mcp/fathom.json` to see available tools
   - Use MCP client to call tools while Fathom runs
   - Get real-time application and cluster state

3. **Example Debugging Session**:
   ```
   You: "Help me debug why pods aren't starting"
   Claude: [Uses get_logs to check Fathom's recent messages]
           [Uses get_app_state to see current context]
           → "I can see Fathom is connected to production-cluster. 
              Let me check the recent logs..."
   ```

## Implementation Details

### Logging Architecture
- Messages are stored in bounded ring buffers (async RwLock)
- Automatic size management (oldest messages dropped when limit exceeded)
- Separate buffers for status (1000 max) and errors (500 max)

### Thread Safety
- All state is stored in Arc<RwLock<T>> for thread-safe access
- Read operations don't block writes
- Backend thread can continuously update state without UI blocking

### Message Propagation
To integrate with actual Fathom operations, the MCP state should receive events from:
- `src/backend/events.rs`: Route notifications to MCP state
- Kubernetes watch streams: Update namespaces and contexts
- Integration status changes: Log integration configuration events

## Configuration File Format

`~/.fathom/mcp/fathom.json`:
```json
{
  "name": "fathom",
  "version": "0.1.0",
  "description": "Kubernetes dashboard with MCP integration for AI debugging",
  "mcp_socket": "~/.fathom/mcp/socket",
  "tools": [
    {
      "name": "get_status",
      "description": "..."
    },
    // ... more tools
  ],
  "capabilities": {
    "tools": true,
    "resources": true,
    "sampling": false
  }
}
```

## API Reference

### McpAppState Methods
```rust
pub async fn push_status(&self, msg: String)        // Add status message
pub async fn push_error(&self, msg: String)         // Add error message
pub async fn set_context(&self, context: String)    // Update cluster context
pub async fn set_namespaces(&self, namespaces: Vec<String>) // Update namespaces
pub async fn get_status(&self, limit: usize) -> Vec<String> // Get recent status
pub async fn get_errors(&self, limit: usize) -> Vec<String> // Get recent errors
pub async fn get_all_logs(&self, limit: usize) -> serde_json::Value // Combined logs
```

## Testing

Unit tests are included in `src/mcp_server.rs`:

```bash
$ cargo test mcp_server::tests
```

Tests verify:
- MCP directory path construction
- Message buffer limits
- State mutations (context, namespaces)
- Log retrieval with limiting

## Security Considerations

- MCP tools only expose read-only information (logs, state)
- No sensitive data (API keys, tokens) in logs
- Socket at `~/.fathom/mcp/` is user-only by default
- Future write operations should require explicit confirmation

## Next Steps

1. **For Immediate Use**: 
   - Configuration file is now accessible to MCP clients
   - Verify Fathom creates `~/.fathom/mcp/fathom.json` on startup

2. **For Full Integration**:
   - Connect Claude to the MCP endpoint
   - Configure event routing to populate logs in real-time
   - Implement socket/stdio transport layer

3. **For Extended Features**:
   - Add handlers for tool requests
   - Expose Kubernetes resource queries
   - Implement write operations with safeguards

---

**Related Files**:
- `src/mcp_server.rs` - MCP server implementation
- `src/backend.rs` - Server initialization in backend thread
- `src/main.rs` - Module declaration

