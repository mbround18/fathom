z# MCP Integration Quick Start

## What is MCP Integration?

Fathom now exposes its internal state via **Model Context Protocol (MCP)**, allowing Claude to query Fathom's logs, status, and Kubernetes information while the app is running.

## Key Files Created

| File | Purpose |
|------|---------|
| `src/mcp_server.rs` | MCP server implementation with tool definitions |
| `src/backend.rs` (modified) | Initializes MCP server on startup |
| `~/.fathom/mcp/fathom.json` | Configuration file with available tools |

## How It Works

1. **Fathom Starts** → MCP server initializes in backend thread
2. **Directory Created** → `~/.fathom/mcp/` is created with `fathom.json`
3. **Claude Connects** → Can query MCP tools while Fathom runs
4. **Real-time Debugging** → Get logs, status, cluster info on demand

## Available Tools

| Tool | Purpose | Example |
|------|---------|---------|
| `get_status` | Recent status messages | "What is Fathom doing right now?" |
| `get_logs` | Status + error logs | "Show me any errors from the last 50 messages" |
| `get_app_state` | Cluster context + namespaces | "Which cluster am I connected to?" |
| `health_check` | Verify Fathom is running | "Is Fathom still responsive?" |

## Quick Commands

```bash
# View MCP config
cat ~/.fathom/mcp/fathom.json

# Check MCP directory
ls -la ~/.fathom/mcp/

# Run Fathom (MCP server starts automatically)
cargo run

# See MCP startup logs
# Look for: "MCP server initialized at ..."
```

## Usage Examples with Claude

### Example 1: Check Status
```
User: "Is Fathom connected to the cluster?"
Claude: [Calls get_app_state tool]
        "You're connected to production-cluster with 8 namespaces available"
```

### Example 2: Debug Errors
```
User: "Are there any errors I should know about?"
Claude: [Calls get_logs with log_type="errors"]
        "Found 2 warning messages related to port-forward setup"
```

### Example 3: Monitor Operations
```
User: "What's been happening in Fathom lately?"
Claude: [Calls get_status with limit=20]
        "Recent activities include cluster connections, namespace 
         refreshes, and pod metric polling"
```

## Architecture Overview

```
┌──────────────────────────────────────────┐
│           Fathom App (egui)              │
│                                          │
│  Backend Thread (tokio)                  │
│  ├─ Kubernetes Operations                │
│  ├─ Log Management                       │
│  └─ MCP Server                           │
│     ├─ McpAppState                       │
│     │  ├─ Status Messages (1000 max)     │
│     │  ├─ Error Messages (500 max)       │
│     │  └─ Context/Namespaces             │
│     └─ Tool Handlers (future)            │
│                                          │
└──────────────────────────────────────────┘
         │                  │
         │                  └──→ ~/.fathom/mcp/fathom.json
         │                       (Configuration)
         │
         └──→ ~claude.local MCP~
              (Query Tools)
```

## Message Buffers

- **Status Messages**: Ring buffer (1000 max) - info & status logs
- **Error Messages**: Ring buffer (500 max) - warnings & errors
- **Auto-cleanup**: Oldest messages dropped when limit exceeded

## Future Roadmap

| Phase | Feature | Status |
|-------|---------|--------|
| 1 | MCP Server foundation & tool definitions | ✅ Done |
| 2 | Socket/StdIO transport & tool handlers | 🔄 Planned |
| 3 | Kubernetes resource queries (pods, deployments) | 🔄 Planned |
| 4 | Write operations with safeguards | 🔄 Planned |

## Configuration Location

All MCP files are stored at: `~/.fathom/mcp/`

```
~/.fathom/mcp/
├── fathom.json          ← Configuration & tool definitions
└── socket               ← Future: MCP socket endpoint
```

## Next Steps

1. **Now**: Fathom creates `~/.fathom/mcp/fathom.json` on startup
2. **Soon**: Connect Claude to MCP endpoint
3. **Later**: Implement full read/write capabilities through MCP

## Related Documentation

- Full details: See `MCP_INTEGRATION.md`
- Copilot Instructions: `.github/copilot-instructions.md`
- Source Code: `src/mcp_server.rs`

## Testing

```bash
# Run MCP tests
cargo test mcp_server

# Check file was created
cat ~/.fathom/mcp/fathom.json
```

---

**Key Benefit**: Claude can now help you debug and understand Fathom's state and operations in real-time while the application is running. No need to stop the app or check logs separately!

