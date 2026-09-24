# MCP Architecture

Fathom exposes cluster context to external, same-machine MCP clients (Claude Code, Claude
Desktop, or any other MCP-compatible agent) over local **stdio** — no network listener.

## Transport

- Built on `rmcp` 0.12 (`server`/`macros` features), `rmcp::transport::io::stdio()`.
- One MCP server per running Fathom process, spawned as a background tokio task.
- Off by default. The operator turns it on from Settings → Cluster Settings → "Available
  Agents" (`SetMcpServerEnabled { enabled: true }`), which persists the preference
  (`~/.fathom/config.json`, `AutoStartConfig.mcp_stdio`) and starts the stdio task
  synchronously in the same command handler. Turning it off cancels the task synchronously;
  no process/listener runs while disabled. `LoadMcpServerStatus` reads back the current
  enabled/running state plus the client connection snippet.
- `--disable-mcp` (set by shells like `fathom-devbridge` that must never expose an MCP
  surface) refuses to start the server regardless of the persisted preference.

## How a client actually attaches: `fathom --only-mcp`

Stdio means the MCP channel *is* a process's stdin/stdout, so the client has to be the one that
launches the server. The connection snippet Settings shows is therefore a command, not an address:

```
/path/to/fathom --only-mcp
```

`--only-mcp` runs the backend headless (no window) and hands that process's own stdio to the
client. `backend::mcp::bootstrap` decides what such a process does, and there are only three
outcomes:

| `--only-mcp` | server enabled | behaviour |
| --- | --- | --- |
| no | no | normal windowed startup, nothing spawned (the default for a stock install) |
| no | yes | server task spawned at startup, then the usual command loop — an enabled server survives a restart |
| yes | no | **refuses on stderr with a stated reason and exits.** A headless process whose only job is a switched-off endpoint would otherwise block forever, which a client can only observe as an unexplained hang |
| yes | yes | connect to the kubeconfig's current context, serve the client, exit when it disconnects |

Three consequences worth knowing:

- **The gate is not bypassable from the command line.** The snippet is `--only-mcp` alone and
  deliberately *not* `--only-mcp --mcp-stdio`: if it forced the server on, any local process could
  attach to the operator's cluster while the Settings toggle said off. `--mcp-stdio`
  (`FATHOM_MCP_STDIO_ENABLED`) exists as a force-on for tests and scripted use, not for the
  snippet.
- **The headless process connects itself.** It has no UI to issue `SwitchContext`, so
  `bootstrap` reads the kubeconfig's current context and connects before starting the server. A
  failure there is not fatal: `tools/list` still answers and each call reports the missing
  connection instead of inventing data.
- **The process exits when its client does.** `McpState::wait` awaits the server task, which ends
  on stdin EOF, so a disconnected client leaves no orphan behind.
- **`tracing` writes to stderr, never stdout** (`startup::init_tracing`). One log line on stdout
  is an unrecoverable JSON-RPC framing error for the attached client.

## Tool surface

### Read tools (RBAC-gated, no approval required)

`get_resource`, `list_resources`, `get_pod_logs`, `get_events`, `describe_pod` (the last reuses
`backend::agent::grounding::collect_pod_grounding`, the same grounding Fathom's in-app agent
panel uses).

### Mutation tools (RBAC-gated *and* resource-version-guarded)

`backend::mcp::tools_mutate` exposes three:

| Tool | Kinds | Verb checked |
|---|---|---|
| `scale_workload` | `Deployment`, `StatefulSet` | `update` |
| `apply_resource_yaml` | any kind the cluster can discover, including CRDs | `update` |
| `delete_resource` | `Pod`, `Deployment`, `StatefulSet` | `delete` |

The kind lists for scale and delete are closed on purpose: they are exactly the kinds that
already have an event-emitting backend mutation function to reuse. An unsupported kind is
refused rather than handled by new bespoke write code.

#### Guard sequence

Every mutation tool handler runs the same four steps, in this order, and no branch reaches a
write without all of them passing:

```
tool call arrives
  0. schema            expected_resource_version is a mandatory, non-Option String with no
                       default, so a call that omits it fails deserialization and no handler
                       body ever runs
  1. RBAC              rbac_guard::resolve_current_access (SelfSubjectReview + the same
                       resolve_namespace_access resolver 011 uses) then rbac_guard::check with
                       the *mutating* verb
                         -> refuse: McpToolDenial{ RbacDenied }, "Nothing was read from the
                            cluster" — the target is never even re-read
  2. version guard     re-GET the live target; compare metadata.resourceVersion (and uid, when
                       the caller pinned one) against what the agent presented
                         -> refuse: McpToolDenial{ MutationGuardRejected }, "Nothing was written
                            to the cluster"
  3. event bus         the backend's event sender must be attached, so the mutation can be
                       reported; checked before the write, never after
  4. write             an existing backend mutation function, which also emits the event
```

Step 2 is what makes the feature safe: the `expected_resource_version` an MCP client sends is
treated as *evidence it read the object*, not as authorization. It is re-validated server side,
against the live cluster object, immediately before the write. Client-side approval UX in Claude
Code / Claude Desktop is an additional layer, never a substitute — Fathom would refuse a stale
call that a client had happily approved. A value that is empty, stale, or belongs to a
since-recreated object (different `uid`) is refused, and a target that can no longer be read is
refused too rather than written blind.

#### One write path, shared with the UI

Step 4 never contains write logic of its own. It calls the same function the equivalent
`BackendCmdV1` handler calls:

| Tool | Function called |
|---|---|
| `apply_resource_yaml` | `backend::apply::apply_resource(..., expected_resource_version)` — the exact function `ApplyResourceYaml` uses, then `apply::emit_resource_applied` |
| `scale_workload` | `backend::deployments::scale_deployment_with_client` / `backend::statefulsets::scale_statefulset_with_client` |
| `delete_resource` | `backend::pods::delete_pod_with_client` / `backend::deployments::delete_deployment_with_client` / `backend::statefulsets::delete_statefulset_with_client` |

The `*_with_client` functions are the bodies of the pre-existing `scale_deployment`,
`delete_pod`, … handlers, with the `kube::Client` and event bus passed in explicitly instead of
borrowed from `BackendState` (which a spawned MCP task cannot hold). The UI-facing functions are
now thin wrappers over them, so there is one write path and one emission per action, not two.

#### The operator's own UI sees it

Because the write and its event emission live together in those shared functions, a successful
MCP mutation emits exactly the `BackendEvent` a UI-driven one does — the same
`SystemEvent::Notification` for scale/delete ("Scaled deployment payments/api to 10"), the same
`StreamEvent::ResourceApplied` for apply. `SetMcpServerEnabled { enabled: true }` publishes the
backend's event sender into the MCP server's cluster handle alongside the `kube::Client`, which
is what lets the spawned task emit at all. An operator watching Fathom therefore sees an
external agent's change live, as required by Constitution Principle I.

Tool metadata also tells compliant clients to ask first: each mutation tool's description states
"This action modifies the cluster. Confirm with the operator before calling." and carries
`read_only_hint: false` / `destructive_hint: true` annotations.

## AI provider sign-in (038)

The in-app assistant's provider is chosen in Settings -> Cluster Settings -> AI Assist
(`backend::ai`). GitHub Models signs in by device flow; Claude and OpenAI take an API key;
Ollama needs no credential. Secrets live only in the OS keyring (service `fathom`, account
`ai-assist/<provider>`); a 0600 file store is an explicit opt-in used only when no keyring
exists. The legacy 013 plaintext `~/.fathom/credentials.json` is migrated write, read back and
verify, then removed. Every provider request goes through one HTTP chokepoint (`AiHttp`); in
offline mode it refuses any non-loopback address and the effective provider is forced to
Ollama with no hosted fallback. These tools are not exposed to MCP clients.

## Authorization

Every tool call is scoped by a `SelfSubjectAccessReview` issued against the same kubeconfig
identity Fathom itself authenticates as (`backend::mcp::rbac_guard`), mirroring the check
`RefreshRbacSelfAccess` already performs elsewhere in the app. A call against a namespace/verb
the connected cluster identity cannot itself perform is denied before any cluster read/write is
attempted.

## Demo chat

The Available Agents section can detect a local `claude` CLI on `PATH` and launch it as a
subprocess pre-configured to attach to Fathom's own MCP server, so the operator can immediately
try an example conversation grounded in the live cluster. If no compatible CLI is detected, the
section says so plainly and never fabricates a chat session.

## What was removed

This design replaces an earlier HTTP-based MCP prototype (`src/mcp_server.rs`, port 18288, no
authentication, an ungated `restart_deployment` mutation tool) that predated this repo's
mutation-gating conventions. It has been fully retired, not kept alongside the new server.
