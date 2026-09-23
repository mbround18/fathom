# Data Model: MCP Agent Exposure

## Entities

### McpServerStatus (derived, not persisted)

Runtime-only status reported to the frontend; not stored on disk.

| Field | Type | Notes |
|---|---|---|
| `enabled` | `bool` | The persisted operator preference (`AutoStartConfig.mcp_stdio`, formerly `mcp_http`). |
| `running` | `bool` | Whether the stdio server task is actually alive right now. Always `false` when `enabled` is `false`. |
| `connectionCommand` | `string` | The literal stdio-attach command/config snippet to paste into an external client's MCP config (e.g. the Fathom binary invoked with an `--mcp-stdio` style flag, or equivalent). |

### McpServerSettings (persisted)

Reuses/repurposes the existing `AutoStartConfig` shape in `~/.fathom/config.json` (`src/backend/persistence.rs:17`). No new file.

| Field | Type | Notes |
|---|---|---|
| `mcp_stdio` | `bool` | Renamed from `mcp_http`. Default `false` (off), per FR-002. |

### McpToolDenial (protocol type, wire shape shared with frontend for display/debugging)

| Field | Type | Notes |
|---|---|---|
| `tool` | `string` | Tool name that was denied. |
| `reason` | `string` | Human-readable reason (RBAC scope, missing/stale resource version, server disabled, no cluster connected). |
| `category` | `enum` | `RbacDenied` \| `MutationGuardRejected` \| `NoClusterConnected` \| `ServerDisabled`. |

### McpMutationRequest (internal, not wire — shape of what a mutation tool call carries)

| Field | Type | Notes |
|---|---|---|
| `target` | `ObjectRef`-equivalent (kind, namespace, name) | Same identity shape used elsewhere in the protocol. |
| `expected_resource_version` | `string` | REQUIRED. Obtained by the calling agent from a prior read-tool response. Re-validated server-side against the live object before any write, exactly like `ApplyResourceYaml`'s `apply_resource(..., expected_resource_version)` (`src/backend/apply.rs`). |
| `desired_change` | tool-specific (e.g. replica count for scale, patch body for apply) | Validated for shape before the guard runs. |

### McpDemoChatSession (runtime, not persisted)

| Field | Type | Notes |
|---|---|---|
| `sessionId` | `u32` | Monotonic id, same "track the newest id you started" convention `PodTerminalTab.tsx`/`PodLogsTab` use for exec/log streams. |
| `state` | `enum` | `Detecting` \| `NotDetected` \| `Starting` \| `Active` \| `Ended` \| `Error`. |
| `cliPath` | `string \| null` | Resolved path to the detected `claude` binary, or `null` if not detected. |

## State Transitions

### MCP server lifecycle

```
disabled (default) --[operator toggles on]--> enabling --[stdio task spawned]--> running
running --[operator toggles off]--> stopping --[task cancelled via CancellationToken]--> disabled
running --[app shutdown]--> disabled
```

The server is never in `running` state while `enabled` is `false` — this is the FR-002 invariant, enforced by starting the stdio task only from the same code path that flips `enabled` to `true`, and cancelling it synchronously (via `rmcp`'s `serve_server_with_ct`, see research.md R1) in the same path that flips it to `false`.

### Mutation tool call

```
tool call received
  -> RBAC gate (SelfSubjectAccessReview via rbac_access.rs-equivalent check) --deny--> McpToolDenied(RbacDenied)
  -> resource-version guard (re-fetch target, compare to expected_resource_version) --mismatch--> McpToolDenied(MutationGuardRejected)
  -> execute via existing mutating command path --> success event (same shape as UI-driven mutation)
```

No branch executes the mutation before both gates pass. This is what `mcp_mutation_tools_require_explicit_approval` asserts in the test suite: constructing a call that skips either gate must be structurally impossible (the mutation function signature requires an already-validated guard token / resource-version match), not just conventionally discouraged.

### Demo chat session

```
Detecting -> NotDetected (no `claude` on PATH) [terminal, shows instructions]
Detecting -> Starting (CLI found + MCP server running) -> Active -> Ended
Starting -> Error (spawn failed, or MCP server toggled off before attach)
```

## Relationships

- `McpServerStatus.enabled` is sourced from `McpServerSettings.mcp_stdio`.
- Every `McpToolDenial` references RBAC state computed via the same `resolve_namespace_access` output already computed for the RBAC administration feature (011) — no separate authorization model.
- `McpDemoChatSession` can only reach `Starting`/`Active` while `McpServerStatus.running` is `true`; if the operator disables the server mid-session, the session transitions to `Ended`.
