# Fathom MCP Integration - Architecture Overview

## Directory Structure

```
fathom/
├── src/
│   ├── main.rs (modified)
│   │   └── Added: mod mcp_server;
│   │
│   ├── backend.rs (modified)
│   │   └── Added: MCP server initialization in init()
│   │
│   ├── mcp_server.rs (NEW)
│   │   ├── McpAppState struct
│   │   │   ├── status_messages: Arc<RwLock<Vec<String>>>
│   │   │   ├── error_messages: Arc<RwLock<Vec<String>>>
│   │   │   ├── current_context: Arc<RwLock<Option<String>>>
│   │   │   └── namespaces: Arc<RwLock<Vec<String>>>
│   │   │
│   │   ├── Functions:
│   │   │   ├── mcp_dir() → ~/.fathom/mcp
│   │   │   ├── mcp_socket_path() → ~/.fathom/mcp/socket
│   │   │   ├── mcp_info_path() → ~/.fathom/mcp/fathom.json
│   │   │   ├── start_mcp_server(state) → Initializes server
│   │   │   └── Tests (unit tests for MCP functionality)
│   │   │
│   │   └── Tool Definitions:
│   │       ├── get_status - Recent status messages
│   │       ├── get_logs - Combined logs
│   │       ├── get_app_state - Cluster state
│   │       └── health_check - Responsiveness check
│   │
│   ├── app/
│   │   └── (UI rendering and state)
│   │
│   ├── backend/
│   │   ├── state.rs
│   │   ├── kube.rs
│   │   ├── pods.rs
│   │   ├── deployments.rs
│   │   ├── events.rs (event broadcasting)
│   │   ├── integrations.rs
│   │   ├── metrics.rs
│   │   └── ... (other backend modules)
│   │
│   └── protocol/
│       └── (Communication types)
│
├── Cargo.toml (modified)
│   └── Added: rmcp = { version = "0.12.0", features = ["server"] }
│
├── Documentation (NEW)
│   ├── MCP_INTEGRATION.md - Full technical documentation
│   ├── MCP_QUICK_START.md - Quick reference guide
│   └── MCP_IMPLEMENTATION_SUMMARY.md - This file
│
└── ~/.fathom/mcp/ (Runtime, auto-created)
    └── fathom.json (Configuration file with tool definitions)
```

## Runtime MCP Directory

When Fathom starts, it creates:

```
~/.fathom/mcp/
├── fathom.json
│   ├── Server metadata
│   ├── Version: 0.1.0
│   ├── Description
│   ├── MCP socket path
│   └── Available tools + schemas
│
└── socket (future)
    └── MCP communication endpoint
```

## Data Flow

### Initialization Flow
```
main.rs
  └→ backend.rs::init()
      └→ Runtime::new()
          └→ MCP server spawned
              └→ mcp_server.rs::start_mcp_server()
                  ├→ Create ~/.fathom/mcp/
                  ├→ Write fathom.json
                  └→ Log initialization
```

### Message Flow (Planned)
```
Backend Operations
  ├→ Kubernetes API calls
  ├→ Pod/Deployment updates
  └→ Integration status changes
      ↓
  Events broadcast via events::notify_*()
      ↓
  McpAppState buffers updated
      ↓
  Claude queries via MCP tools
      ↓
  Real-time debugging & assistance
```

## Thread Model

```
┌─────────────────────────────────────┐
│   Main Thread (UI/eframe)           │
│   - Renders UI                      │
│   - Polls mailbox (non-blocking)    │
│   - Dispatches commands             │
└─────────────────────────────────────┘
          ↑                    ↓
    Channels (crossbeam)
          ↓                    ↑
┌─────────────────────────────────────┐
│   Backend Thread (tokio runtime)    │
│   - Kubernetes operations           │
│   - Async tasks                     │
│   - Event broadcasting              │
│   - MCP Server                      │
│     ├─ McpAppState (Arc<RwLock>)   │
│     ├─ Status/Error buffers        │
│     └─ Tool definitions            │
└─────────────────────────────────────┘
```

## Event Broadcasting (Integration Point)

When events are broadcast via `events::notify_*()`:

```
events::emit(...)
  ├→ Sends to UI (across channel)
  └→ Updates MCP state (future)
      └→ Pushes to status_messages or error_messages buffer
```

This ensures Claude always has access to:
- Recent application status
- Any errors or warnings
- Current Kubernetes context
- Available namespaces

## API Boundaries

### Public API (mcp_server.rs)

```rust
pub struct McpAppState {
    pub status_messages: Arc<RwLock<Vec<String>>>,
    pub error_messages: Arc<RwLock<Vec<String>>>,
    pub current_context: Arc<RwLock<Option<String>>>,
    pub namespaces: Arc<RwLock<Vec<String>>>,
}

impl McpAppState {
    pub async fn push_status(&self, msg: String)
    pub async fn push_error(&self, msg: String)
    pub async fn set_context(&self, context: String)
    pub async fn set_namespaces(&self, namespaces: Vec<String>)
    pub async fn get_status(&self, limit: usize) -> Vec<String>
    pub async fn get_errors(&self, limit: usize) -> Vec<String>
    pub async fn get_all_logs(&self, limit: usize) -> JsonValue
}

pub fn mcp_dir() -> Result<PathBuf>
pub fn mcp_socket_path() -> Result<PathBuf>
pub fn mcp_info_path() -> Result<PathBuf>
pub async fn start_mcp_server(state: McpAppState) -> Result<()>
```

## Integration Points (For Future Enhancement)

### 1. Event Broadcasting
Location: `src/backend/events.rs`

```rust
// Route notifications to MCP state
events::notify_info(...) 
  └→ mcp_state.push_status(...)
events::notify_error(...)
  └→ mcp_state.push_error(...)
```

### 2. Context/Namespace Updates
Location: `src/backend/kube.rs`

```rust
// After loading contexts
mcp_state.set_context(context.clone()).await;

// After loading namespaces
mcp_state.set_namespaces(namespaces).await;
```

### 3. Tool Request Handlers
Location: `src/mcp_server.rs` (when transport available)

```rust
async fn handle_tool_request(
    tool_name: &str,
    args: JsonValue,
    state: &McpAppState,
) -> Result<JsonValue> {
    match tool_name {
        "get_status" => { /* implementation */ },
        "get_logs" => { /* implementation */ },
        "get_app_state" => { /* implementation */ },
        "health_check" => { /* implementation */ },
        _ => Err("Unknown tool"),
    }
}
```

## Configuration (fathom.json)

```json
{
  "name": "fathom",
  "version": "0.1.0",
  "description": "Kubernetes dashboard with MCP integration for AI debugging",
  "mcp_socket": "~/.fathom/mcp/socket",
  "tools": [
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
    }
  ],
  "capabilities": {
    "tools": true,
    "resources": true,
    "sampling": false
  }
}
```

## Performance Characteristics

- **Memory**: Bounded buffers (1000 status + 500 error messages max)
- **CPU**: Minimal overhead (log buffering only)
- **Concurrency**: Lock-free reads, short write locks
- **Scalability**: No impact on UI/Kubernetes operations

## Security Model

```
┌──────────────────┐
│   Fathom App     │
│  (No changes)    │
└──────────────────┘
         ↓
┌──────────────────────────────────────┐
│   MCP State (Read-Only)              │
│   - Status/Error logs only           │
│   - No API keys or secrets           │
│   - No sensitive data                │
└──────────────────────────────────────┘
         ↓
┌──────────────────┐
│   Claude/MCP     │
│   (Queries only) │
└──────────────────┘
```

## Testing Strategy

### Unit Tests
- `tests/mcp_socket_path` - Path construction
- `tests/mcp_dir` - Directory path
- `tests/app_state` - State mutations
- `tests/message_buffer_limit` - Buffer management
- `tests/get_status` - Log retrieval

Run: `cargo test mcp_server`

### Integration Tests (Future)
- Connect MCP client and verify tool availability
- Validate response formats
- Test concurrent state updates
- Verify buffer limits under load

## Development Workflow

### Phase 1: Foundation (✅ Complete)
1. Create MCP server infrastructure
2. Define tool schemas
3. Set up directory/config file
4. Write documentation

### Phase 2: Transport (🔄 In Progress)
1. Implement rmcp stdio transport
2. Add tool request handlers
3. Hook up event routing
4. Test with MCP client

### Phase 3: Features (📅 Planned)
1. Expose Kubernetes resources
2. Add metrics queries
3. Implement write safeguards
4. Create operation audit trail

### Phase 4: Production (📅 Future)
1. Performance optimization
2. Security hardening
3. Rate limiting for tools
4. Audit logging

## Troubleshooting

### MCP directory not created
- Check filesystem permissions in ~/.fathom/
- Verify Fathom runs with proper privileges
- Check logs for errors

### fathom.json empty
- Wait for Fathom to fully initialize
- Check backend thread is running
- Verify tokio runtime created successfully

### Tool schema validation fails
- Ensure serde_json serialization works
- Check tool parameter types
- Verify JSON schema compliance

---

**Last Updated**: December 30, 2025
**Status**: Foundation Phase Complete ✅

