# Protocol Contract: MCP Agent Exposure

Follows this repo's established convention (see 011/013/014 `contracts/protocol.md`): new
`BackendCmdV1` / `BackendEventV1` variants live in `src/protocol/v1/{cmd.rs,event.rs}`, each with
a `mutation_target()` arm in `mutation.rs`, and a round-trip serde test in `tests/protocol_serde.rs`.

## New commands (`src/protocol/v1/cmd.rs`)

```rust
LoadMcpServerStatus,                       // ReadOnly — fetch current enabled/running + connection snippet
SetMcpServerEnabled { enabled: bool },     // Local — toggles the persisted preference and starts/stops the stdio task
StartMcpDemoChat,                          // Local — spawns the detected `claude` CLI attached to the MCP server
SendMcpDemoChatInput { data: String },     // Local — base64 input, same encoding convention as SendPodExecInput
StopMcpDemoChat,                           // Local
```

Notes:
- `SetMcpServerEnabled` is `Local` in `mutation_target()` — it never touches the cluster, matching
  `mutation.rs`'s own doc comment (`Local` already explicitly lists "the MCP server" as an example).
- No new *cluster-mutating* commands are added to the UI-facing protocol — MCP mutation tools
  (scale/apply/delete) are invoked by the external MCP client via `rmcp`'s own `call_tool`
  dispatch, not via `BackendCmdV1`. Internally, the MCP mutation-tool handlers call the *same*
  Rust functions the existing `ApplyResourceYaml`/`ScaleWorkload`/etc. `BackendCmdV1` handlers
  call (e.g. `apply_resource(..., expected_resource_version)`), so the guard code path is shared,
  not duplicated — see data-model.md's "Mutation tool call" state transition.

## New events (`src/protocol/v1/event.rs`)

```rust
McpServerStatusChanged { enabled: bool, running: bool, connection_command: String },
McpDemoChatReady { session_id: u32 },
McpDemoChatOutputChunk { session_id: u32, data: String }, // base64, same shape as PodExecOutputChunk
McpDemoChatEnded { session_id: u32, reason: String },
McpDemoChatNotDetected { instructions: String },
```

`SystemEvent::UiConfigLoaded`'s existing `mcp_auto_start: bool` field (`event.rs:156`) is renamed
`mcp_stdio_enabled` in place — same wire position/semantics, now describing the new stdio server
instead of the deleted HTTP one (research.md R4).

## Removed (as part of retiring `src/mcp_server.rs`)

Any `BackendCmdV1`/`BackendEventV1` variants that existed solely to open the old MCP config
directory or report the old HTTP server's status are removed; confirm exact variant names during
implementation by grepping `cmd.rs`/`event.rs` for `Mcp` before editing (this plan does not
assume variant names that turn out not to exist — see tasks.md for the verification step).

## MCP tool surface (external protocol, not `BackendCmdV1` — this is what `rmcp`'s `ServerHandler`
exposes to an attached client)

### Read tools (RBAC-gated, no approval required)

- `get_resource(kind, namespace, name)` → structured resource summary
- `list_resources(kind, namespace?)` → filtered by `resolve_namespace_access`-derived scope
- `get_pod_logs(namespace, name, container?, lines?)`
- `get_events(namespace?, involvedObject?)`
- `describe_pod(namespace, name)` — reuses `grounding.rs::collect_pod_grounding`

### Mutation tools (RBAC-gated AND resource-version-guarded; tool schema marks them consequential)

- `scale_workload(kind, namespace, name, replicas, expected_resource_version)`
- `apply_resource_yaml(yaml, expected_resource_version)`
- `delete_resource(kind, namespace, name, expected_resource_version)`

Every mutation tool's JSON schema requires `expected_resource_version` as a mandatory field (no
default), which forces any compliant MCP client to have first called a read tool to obtain it —
this is the mechanical link between "read, then approve, then mutate" and is re-verified
server-side (not trusted) per FR-004/data-model.md.

## Testing contract

- `tests/protocol_serde.rs`: round-trip every new `BackendCmdV1`/`BackendEventV1` variant above.
- `src/backend/mcp/tools_mutate.rs` (or equivalent): `mcp_mutation_tools_require_explicit_approval`
  — construct a mutation tool call with a stale/absent `expected_resource_version` and assert no
  cluster write occurs (fixture/fake client, no live cluster).
- `src/backend/mcp/rbac_guard.rs` (or equivalent): `mcp_tool_calls_respect_namespace_rbac` —
  fixture-based, modeled on `rbac_access.rs`'s existing test module, asserting an out-of-scope
  namespace tool call is denied with a specific reason and never reaches the cluster client call.
