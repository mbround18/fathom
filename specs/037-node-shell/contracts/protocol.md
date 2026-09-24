# Contract: Node Shell protocol (v1, additive)

Location: `src/protocol/v1/cmd.rs`, `event.rs`, `mutation.rs`. Bindings regenerate; drift is a CI failure. Additive, so no protocol version bump.

## Commands (UI -> backend)
| Command | Data | Class |
|---|---|---|
| `CheckNodeShellAccess` | `{ namespace }` | ReadOnly |
| `StartNodeShell` | `{ node, namespace, image, image_pull_secret?, deadline_seconds? }` | `Namespaced { namespaces: [namespace] }` (writes_cluster = true) |
| `StopNodeShell` | `{ session_id, namespace }` | `Namespaced` (deletes a pod) |
| `ListNodeShellOrphans` | `{ namespace? }` | ReadOnly |
| `DeleteNodeShellOrphans` | `{ pods: [{ namespace, name }] }` | `Namespaced` |
| `GetNodeShellSettings` / `SetNodeShellSettings` | settings | ReadOnly / local persistence only |

Terminal I/O reuses `SendPodExecInput`, `ResizePodExec` and the exec output events from 002 against the active session. `StopPodExec` alone does NOT delete the pod; only `StopNodeShell` does, and the UI always calls it.

## Events (backend -> UI)
- `NodeShellAccessChecked { namespace, checks[] }`
- `NodeShellStatus { session_id, phase, node, namespace, pod, detail?, leftover }` emitted on every transition, reflecting real cluster state.
- `NodeShellOrphans { pods[] }`
- Failures of generic commands use existing `CommandFinished { Err(verbatim) }`.

## Mutation classification
`StartNodeShell{namespace,..}.mutation_target()` returns `Namespaced`; the `fathom-devbridge` guard refuses it in sandbox mode. Not exposed via MCP.

## Tests required (Constitution III)
Serde round-trip for every variant; `mutation_target` unit tests; pod-spec builder unit test (privileged, host namespaces, nodeName, tolerations, labels, deadline); state-machine unit tests; Playwright specs against the fake backend for each user story.
