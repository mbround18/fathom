# Phase 0 Research: MCP Agent Exposure

## R1: MCP transport crate — `rmcp` vs hand-rolled

**Decision**: Use `rmcp` 0.12 (already declared in `Cargo.toml`, `features = ["server"]`, currently unused) for the new stdio server. Do not hand-roll JSON-RPC dispatch.

**Findings** (read directly from `~/.cargo/registry/src/.../rmcp-0.12.0/src/`):
- Tool definition: `#[tool_router]` on an `impl Block`, `#[tool(description = "...")]` on each async method (`src/lib.rs` doc example, `src/handler/server/router/tool.rs`). A struct implementing the tool set holds a `tool_router: ToolRouter<Self>` field built via `Self::tool_router()`.
- Server trait: `handler::server::ServerHandler` (`src/handler/server.rs:110`) — implement this for the top-level server type; `#[tool_router]`-derived methods get wired into it via `ToolRouter`.
- Transport: `rmcp::transport::io::stdio()` (`src/transport/io.rs:4`) returns `(tokio::io::Stdin, tokio::io::Stdout)`, feeds directly into `rmcp::service::server::serve_server(handler, (stdin, stdout))` (`src/service/server.rs:102`) or `serve_server_with_ct(..., CancellationToken)` (`:160`) for graceful shutdown — the latter is what we use so the Settings toggle can cleanly stop the listener.
- Results/errors: `CallToolResult` (success/error content) and `ErrorData`/`McpError` (aliased) are the standard return types for tool methods, per the crate's own doc example.
- Approval concept: **rmcp has no built-in "requires confirmation" primitive.** A tool's `description` and its declared input schema are the only signal to the client; some MCP clients (Claude Code, Claude Desktop) surface every tool call for interactive approval by default, but this is client policy, not a protocol guarantee. This confirms spec FR-004's requirement that Fathom's backend must independently re-validate — the crate does not do this for us.

**Alternatives considered**:
- Hand-rolled JSON-RPC over stdio (what `src/mcp_server.rs` did over HTTP): rejected — duplicates protocol framing/spec-compliance work `rmcp` already solved, and this session's own dependency list already anticipated using it (`rmcp` was added but never wired up).
- Networked transport (HTTP/SSE, which `rmcp` also supports): explicitly out of scope per spec FR-001 (stdio only, v1).

## R2: Disposition of the pre-existing HTTP MCP prototype

**Decision**: Delete `src/mcp_server.rs`'s HTTP tool-call surface entirely, per the coordinator's explicit "Replace" decision (see spec Assumptions). This includes the ungated `restart_deployment_tool` (`src/mcp_server.rs:577-608`), which patches a Deployment directly from a single tool call with no confirmation and no RBAC check — a real security gap that predates and conflicts with this session's mutation-gating conventions (013's `agent_commands_never_write_cluster`, `mutation.rs`'s guard pattern).

**Call sites requiring clean unwiring** (confirmed via `grep -rli mcp src/`):
- `src/lib.rs:7` — `pub mod mcp_server;` (unconditional, no feature gate)
- `src/main.rs`, `src/backend.rs`, `src/backend/state.rs` — `McpAppState` wiring, `initialize_mcp_server()`/`run_http_mcp_server()` calls
- `src/backend/cmd/integrations.rs`, `src/backend/cmd/cluster.rs`, `src/backend/deployments.rs` — push status/error messages and deployment cache into `McpAppState`
- `src/protocol/v1/{cmd.rs,event.rs,mutation.rs,route.rs,ui_config.rs}` — the `mcp_auto_start` field/event plumbing (see R3 — this *specific* piece is repurposed, not deleted)
- `src/backend/persistence.rs` — `AutoStartConfig.mcp_http`, `load_mcp_auto_start`/`save_mcp_auto_start` (repurposed, see R3)

## R3: `mcp_integration.rs` (egui settings view) — retire

**Decision**: Delete `src/app/views/mcp_integration.rs` and its registration in `src/app/mod.rs` (`McpIntegrationView`, `views::mcp_integration` import, the view-switch arm at `src/app/mod.rs:226`).

**Finding**: `src/app/mod.rs` — and therefore everything under `src/app/views/`, including `mcp_integration.rs` — is gated behind `#[cfg(feature = "egui-ui")]` at `src/lib.rs:2-3`. `ui/src-tauri/Cargo.toml:20` depends on the root crate with `fathom = { path = "../..", default-features = false }`, which disables `egui-ui` (the crate's only default feature). **The current Tauri app never compiles or reaches `src/app/*` at all.** `mcp_integration.rs` is reachable only from the standalone legacy `fathom` binary (`[[bin]] name = "fathom", required-features = ["egui-ui"]`), which the repo keeps buildable for parity reasons (Cargo.toml FR-025 comment), not because it's load-bearing for the shipped Tauri app.

Since `mcp_integration.rs`'s entire purpose is displaying/toggling the HTTP server being deleted in R2, leaving it in place would break the `egui-ui` build (dangling references to `McpAppState`/`mcp_server::*`). It is deleted along with its registration so the legacy egui binary keeps building green, per the standing "cargo build must be clean" bar — it is not replaced with an equivalent egui view for the new stdio server, since the egui app is legacy-parity-only and Settings for this feature lives in the current Tauri UI (spec FR-007, home is `ClusterSettingsPanel.tsx`).

## R4: MCP on/off toggle — persistence

**Decision**: Reuse `src/backend/persistence.rs`'s `AutoStartConfig` struct and its `load()`/`save()` pattern (`persistence.rs:17-49`) — this is the exact existing mechanism for a small operator-controlled boolean preference outside per-context `ClusterSettings`. Rename/repurpose the existing `mcp_http: bool` field to `mcp_stdio: bool` (the old field's only purpose was toggling the now-deleted HTTP server) rather than adding a parallel storage file.

**Why not `ClusterSettings` (014's per-context settings)**: `src/protocol/v1/team.rs:57-59` explicitly documents `ClusterSettings` as "§10's field set, exactly — this feature MUST NOT add any other preference category here (FR-006)" (014's own constraint). The MCP server on/off state is also not meaningfully per-kube-context (it's a single local process serving whichever context is currently active), so `AutoStartConfig`'s existing global-preference file (`~/.fathom/config.json`, loaded via `load_ui_config()`/`UiConfig.mcp_auto_start`) is the correct home, already established for precisely an MCP-related boolean.

**Consequence**: `src/protocol/v1/event.rs:156`'s `mcp_auto_start: bool` field (part of `SystemEvent::UiConfigLoaded`) is kept, renamed `mcp_stdio_enabled`, and now reflects/drives the new stdio server instead of the old HTTP one — the wire shape and load/save plumbing are unchanged, only what the boolean controls changes. This avoids inventing new protocol surface for the same concept, per the coordinator's brief.

## R5: RBAC scoping

**Decision**: Call `src/backend/rbac_access.rs::resolve_namespace_access` (`rbac_access.rs:302`, `pub(super)`) directly from the new MCP module. Visibility check: `pub(super)` on an item in `backend::rbac_access` resolves to `pub(in crate::backend)`, which is visible to any descendant module of `crate::backend` — so a new `crate::backend::mcp` module can call it without any visibility changes.

**Approach**: Every MCP tool call (read or mutate) resolves the current kube context's namespace access via this function before touching the cluster, and denies out-of-scope namespace targets with the same messaging style as 011's namespace-access-denial UX.

## R6: Read tool surface

**Decision**: Build MCP read tools on top of the same data-assembly logic already used by `src/backend/agent/grounding.rs::collect_pod_grounding` (`pub(crate)`, `grounding.rs:28`) for pod-shaped context, and the existing read command handlers under `src/backend/cmd/*` for other resource kinds (get/list/describe/logs/events), rather than reimplementing cluster reads. `grounding.rs`'s function is reused directly for the pod-context read tool; other read tools call the same backend functions the existing `BackendCmd` read variants already call.

## R7: Credential/auth detection reuse

**Decision**: Reuse `src/backend/agent/credentials.rs::load_agent_auth`/`load_for_request` (`pub(crate)`, `credentials.rs:36,52`) to populate the Settings section's "Anthropic/Ollama already configured" display — no new detection logic. Frontend reuses whatever store slice `ui/src/components/panels/agent-ops/AuthSetup.tsx` already reads, surfaced read-only in the new "Available Agents" section (this feature does not let the operator edit agent auth from here — that stays AuthSetup's job).

## R8: Demo chat launch / terminal reuse

**Decision**: Fathom already has a real interactive terminal component, `ui/src/components/panels/PodTerminalTab.tsx`, built on `xterm.js` + a backend PTY-like exec session (`StartPodExec`/`SendPodExecInput`/`ResizePodExec`/`StopPodExec`, `src/backend/exec.rs`) — but that pathway is specifically `kube exec` into a pod, not a local host subprocess. Launching a local `claude` CLI process is a different primitive (host process, not cluster exec), so this spec adds a small, parallel local-process session type (new `StartMcpDemoChat`/`StopMcpDemoChat` commands + output-chunk event) that reuses the *same frontend xterm.js rendering pattern* `PodTerminalTab.tsx` established (same library, same chunked-output/resize contract shape) rather than the pod-exec backend plumbing itself, since the process being attached to is local, not in-cluster. This keeps Principle II intact (cluster access stays backend-owned and separate) while not duplicating xterm wiring conventions on the frontend.

**CLI detection**: a `which claude` (or platform-equivalent PATH search) check on demand, never cached/fabricated; if absent, the Settings section shows the "not detected" state per spec FR-011.

## R9 (implementation-time correction): reuse existing MCP command/event names instead of new ones

During implementation, inventory of the actual worktree `cmd.rs`/`event.rs`/`mutation.rs`/`route.rs` showed the old HTTP prototype already had a complete, sensible command surface: `StartMcpServer`, `StopMcpServer`, `RefreshMcpStatus`, `OpenMcpConfigDir`, `SetMcpAutoStart { enabled: bool }`, and event `McpServerStatus { running: bool, message: String }`, plus `UiConfigLoaded.mcp_auto_start: bool`. These names are already transport-agnostic (nothing HTTP-specific in the wire names themselves). Rather than retire this surface and invent `LoadMcpServerStatus`/`SetMcpServerEnabled`/etc. as contracts/protocol.md originally sketched (written before this inventory), implementation repoints the *existing* command handlers at the new stdio server:

- `SetMcpAutoStart { enabled }` becomes the one master enable/disable toggle (persists via `AutoStartConfig.mcp_stdio`, renamed from `mcp_http`; starts/stops the stdio task synchronously).
- `StartMcpServer`/`StopMcpServer`/`RefreshMcpStatus` remain manual controls issued by the Settings UI and CLI startup path.
- `McpServerStatus` event gains two new fields: `enabled: bool` and `connection_command: String`, so the Settings section has everything in one event (rather than adding a second `LoadMcpServerStatus`-only path).
- `OpenMcpConfigDir` is kept, repointed at the new `src/backend/mcp/mod.rs::mcp_dir()` (same `~/.fathom/mcp/` location), used to house the generated connection-command snippet file.
- `UiConfigLoaded.mcp_auto_start` field is **not** renamed (no protocol churn needed — the name was already transport-agnostic).

This is strictly less protocol surface than the original draft while satisfying the same requirements (FR-002 default-off, FR-007 Settings visibility, FR-013 immediate stop-listening-when-off). Demo chat commands/events (`StartMcpDemoChat`, `SendMcpDemoChatInput`, `StopMcpDemoChat`, `McpDemoChatReady`, `McpDemoChatOutputChunk`, `McpDemoChatEnded`, `McpDemoChatNotDetected`) remain net-new as planned, since no prior surface existed for them.
