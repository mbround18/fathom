# Quickstart: MCP Agent Exposure

## Prerequisites

- Fathom built from this branch, with a working kube context connected.
- For the demo-chat scenario: the `claude` CLI installed and on `PATH`.

## Scenario 1 — turn the server on and read cluster state externally

1. Open Fathom, go to Settings → Cluster Settings → "Available Agents".
2. Confirm the MCP server shows **off** by default (FR-002). No `fathom`-related stdio-MCP
   process should be attached to anything yet.
3. Toggle it **on**. The section should now show **running**, plus a connection command snippet.
4. In a separate terminal, configure an MCP client (e.g. Claude Code's `mcp` config) to launch
   that exact command, or run it directly and speak MCP `initialize` over stdin/stdout.
5. Call `list_tools` — expect read tools (`get_resource`, `list_resources`, `get_pod_logs`,
   `get_events`, `describe_pod`) and mutation tools (`scale_workload`, `apply_resource_yaml`,
   `delete_resource`).
6. Call `list_resources(kind: "Pod", namespace: <a namespace your kube context can see>)` —
   expect live pod data matching what Fathom's own UI shows for that namespace.
7. Call the same tool for a namespace your kube context does **not** have access to (if your test
   cluster has RBAC restrictions configured) — expect a denial with a specific reason, not the
   data and not a generic error (FR-005/SC-003).

## Scenario 2 — gated mutation

1. With the server running, call `get_resource(kind: "Deployment", namespace: ..., name: ...)` to
   obtain its current `resourceVersion`.
2. Call `scale_workload(..., expected_resource_version: <that value>)` — expect success, and the
   same kind of event Fathom's own UI would show for a UI-driven scale action.
3. Repeat step 2 with a deliberately stale `expected_resource_version` (e.g. the value from before
   step 2's mutation) — expect a clear rejection, and confirm (e.g. via `kubectl get`) the
   cluster was not further mutated by the rejected call (SC-002).

## Scenario 3 — toggle off mid-session

1. With a client attached and mid-conversation, toggle the server **off** in Settings.
2. The next tool call from the still-attached client should fail cleanly (connection closed or a
   clear error), and Settings should show **not running** (FR-013).
3. Confirm (e.g. `ps`/`lsof`) no orphaned stdio server process remains (SC-004).

## Scenario 4 — demo chat launch

1. With the server on and `claude` on `PATH`, click "Launch example chat" in the Available Agents
   section.
2. Expect a real subprocess to start, visibly attached to Fathom's MCP server (e.g. it can answer
   a question about the currently-inspected cluster).
3. Rename/hide the `claude` binary from `PATH` (or test on a machine without it) and reload the
   section — expect the honest "not detected" state with setup instructions, never a fake/mocked
   chat window (FR-011/SC-005).

## Automated coverage entry points

- `cargo test --test protocol_serde` — new command/event round-trip coverage.
- `cargo test mcp_mutation_tools_require_explicit_approval` — mutation guard proof.
- `cargo test mcp_tool_calls_respect_namespace_rbac` — RBAC scoping proof.
- `pnpm -C ui test` — `AvailableAgentsSection` component tests.
- `pnpm -C ui exec playwright test mcp-` — e2e coverage for status/toggle/demo-launch states.
