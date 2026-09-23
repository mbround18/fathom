# Implementation Plan: MCP Agent Exposure

**Branch**: `015-mcp-agent-exposure` | **Date**: 2026-09-23 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `specs/015-mcp-agent-exposure/spec.md`

## Summary

Expose Fathom's cluster context to external, same-machine MCP clients (Claude Code, Claude Desktop) over local stdio, using the already-declared-but-unused `rmcp` crate. The server offers read tools built on the same data assembly the in-app Agent panel already uses (`grounding.rs`, existing read command handlers) and gated mutation tools that funnel through the exact same optimistic-concurrency guard `ApplyResourceYaml` already uses (`apply_resource(..., expected_resource_version)`), plus RBAC scoping via the existing `resolve_namespace_access`. The server is off by default, operator-toggled from a new "Available Agents" section in `ClusterSettingsPanel.tsx`, persisted via the existing `AutoStartConfig` preference file (repurposing the field that used to gate the now-deleted HTTP MCP prototype). That prototype (`src/mcp_server.rs`, `src/app/views/mcp_integration.rs`, `docs/mcp/*.md`) is removed as part of this feature, not kept alongside the new server.

## Technical Context

**Language/Version**: Rust 2024 edition (backend), TypeScript/React (Tauri UI, `ui/`)

**Primary Dependencies**: `rmcp` 0.12 (stdio MCP transport + tool routing, newly wired), existing `kube`, `tokio`, `serde`; frontend: existing Zustand store, `xterm.js` (already a dependency via `PodTerminalTab.tsx`)

**Storage**: `~/.fathom/config.json` via `AutoStartConfig`/`UiConfig` (existing `persistence.rs` mechanism) for the on/off toggle — no new storage file

**Testing**: `cargo test` (protocol serde round-trip in `tests/protocol_serde.rs`, unit tests co-located with new modules), `pnpm -C ui test` (vitest), Playwright e2e (`mcp-` prefixed spec)

**Target Platform**: Desktop (Linux/macOS/Windows), local-machine-only MCP transport (stdio subprocess), no network listener

**Project Type**: Desktop app (Tauri v2 + Rust backend), existing single-repo structure

**Performance Goals**: MCP tool calls should complete within the same latency budget as their equivalent existing `BackendCmd` read/mutate paths (no new perf goal introduced)

**Constraints**: No network-exposed transport (stdio only); mutation tools must never execute without passing the existing `expected_resource_version` guard; server must not start any process/listener while the operator toggle is off

**Scale/Scope**: One MCP server process per running Fathom instance, at most one concurrently-attached external client assumed for v1 (per spec Assumptions)

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

- **Principle I (Operator-First, Explicit Mutation)**: MCP mutation tools require the target's `expected_resource_version` and route through the same guard as `ApplyResourceYaml`; the operator sees the same event-based post-mutation confirmation as every other mutation path. The server itself is off by default and only runs when the operator explicitly enables it. **PASS.**
- **Principle II (One Contract, Backend Owns the Cluster)**: The constitution already names "the MCP server" as backend-owned, long-running work (see constitution.md's Principle II rationale) — this feature is the first to actually build it. All cluster access stays in the Rust backend; the MCP tool layer is a thin adapter over existing `BackendCmd`-equivalent logic, not a new cluster-access path. The stdio server's own new command/event pairs (start/stop, status) go through `src/protocol/v1/` like everything else. **PASS.**
- **Principle III (Tested at the Boundaries)**: New command/event variants get serde round-trip tests in `tests/protocol_serde.rs`; the mutation guard and RBAC scoping each get a dedicated fixture-based unit test (`mcp_mutation_tools_require_explicit_approval`, `mcp_tool_calls_respect_namespace_rbac`) mirroring 013's `agent_commands_never_write_cluster` and 011's `rbac_access.rs` test module. Frontend gets component tests for the new Settings section and `mcp-`-prefixed e2e coverage. **PASS.**
- No violations requiring the Complexity Tracking table.

## Project Structure

### Documentation (this feature)

```text
specs/015-mcp-agent-exposure/
├── plan.md              # This file
├── research.md          # Phase 0 output
├── data-model.md         # Phase 1 output
├── quickstart.md         # Phase 1 output
├── contracts/
│   └── protocol.md       # Phase 1 output
└── tasks.md               # Phase 2 output (/speckit-tasks)
```

### Source Code (repository root)

```text
src/
├── backend/
│   ├── mcp/                     # NEW module — stdio MCP server
│   │   ├── mod.rs               # lifecycle: start/stop, ties into rmcp::ServerHandler
│   │   ├── tools_read.rs        # read tools (pods/deployments/logs/events via existing cmd handlers + grounding.rs)
│   │   ├── tools_mutate.rs      # gated mutation tools (scale/apply/delete) via apply.rs's guard
│   │   ├── rbac_guard.rs        # wraps rbac_access::resolve_namespace_access for tool-call scoping
│   │   └── demo_chat.rs         # local agent-CLI subprocess launch (PATH detection + spawn)
│   ├── cmd/
│   │   └── mcp.rs               # NEW: BackendCmd handlers for StartMcpServer/StopMcpServer/status + demo chat start/stop
│   ├── rbac_access.rs           # UNCHANGED — resolve_namespace_access reused as-is
│   ├── agent/
│   │   ├── grounding.rs         # UNCHANGED — collect_pod_grounding reused as-is
│   │   └── credentials.rs       # UNCHANGED — load_agent_auth reused as-is
│   ├── apply.rs                  # UNCHANGED — apply_resource(...expected_resource_version) reused as-is
│   ├── persistence.rs            # MODIFIED — AutoStartConfig.mcp_http -> mcp_stdio, repurposed
│   ├── deployments.rs            # MODIFIED — remove McpAppState deployment-cache push
│   ├── cmd/{integrations.rs,cluster.rs} # MODIFIED — remove McpAppState status/error push
│   ├── state.rs                  # MODIFIED — remove McpAppState field, add new MCP server handle
│   └── mcp_server.rs             # DELETED (old HTTP prototype)
├── app/views/mcp_integration.rs  # DELETED (egui view for the deleted HTTP prototype)
├── app/mod.rs                    # MODIFIED — remove McpIntegrationView registration
├── lib.rs                        # MODIFIED — remove `pub mod mcp_server;`
├── main.rs, backend.rs           # MODIFIED — remove old MCP server init/shutdown calls
└── protocol/v1/
    ├── cmd.rs                    # MODIFIED — new StartMcpServer/StopMcpServer/StartMcpDemoChat/... variants
    ├── event.rs                  # MODIFIED — McpServerStatusChanged event, mcp_auto_start -> mcp_stdio_enabled rename
    ├── mutation.rs                # MODIFIED — mutation_target() arms for new commands (Local/ReadOnly as appropriate)
    ├── route.rs, ui_config.rs     # MODIFIED — drop old HTTP-server routing/config references
    └── mcp.rs                     # NEW — MCP-specific protocol types shared with frontend (status, connection snippet)

docs/mcp/                          # RETIRED — old docs rewritten to describe new stdio architecture (single MCP_ARCHITECTURE.md kept, others removed as redundant)

ui/src/
├── components/panels/
│   ├── ClusterSettingsPanel.tsx   # MODIFIED — new "Available Agents" section mounted here
│   └── agent-ops/AvailableAgentsSection.tsx  # NEW — toggle, connection info, auth status, demo chat launch
├── store/handlers/                # MODIFIED — new event handlers for MCP status/demo chat output
└── protocol/generated/            # regenerated via `cargo test --test export_bindings`

tests/
├── protocol_serde.rs               # MODIFIED — round-trip tests for new command/event variants
└── (new) backend unit tests co-located under src/backend/mcp/*.rs `#[cfg(test)]` modules

ui/tests/
├── unit/                           # NEW — AvailableAgentsSection component tests
└── e2e/mcp-agent-exposure.spec.ts  # NEW — mcp- prefixed e2e coverage
```

**Structure Decision**: Single-project desktop app structure (existing). New backend work lives under `src/backend/mcp/` as a sibling to `src/backend/agent/`, mirroring how 013 organized the in-app agent logic — deliberately kept separate from `agent/` since this is an outbound-facing surface (non-goal: do not modify `AgentPanel.tsx`/`agent/state_machine.rs`), even though it reuses `agent/grounding.rs` and `agent/credentials.rs` as read-only dependencies.

## Complexity Tracking

*No constitution violations requiring justification.*
