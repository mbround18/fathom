# Data Model: Pod Exec Session Gate

This spec adds one new request/response pair to the existing protocol (`src/protocol/v1/`)
and one new phase to the existing `ExecStreamState`/`PodTerminalTab` client state (spec 002).
No new persisted entities; nothing here survives a restart.

## Backend protocol additions

### `BackendCmdV1::CheckPodExecAccess` (new command)

```rust
CheckPodExecAccess {
    namespace: String,
    name: String,
    /// The container the operator has chosen (or the pod's sole/default container).
    /// Carried through for completeness/logging; the access review itself is not
    /// container-scoped (Kubernetes RBAC does not gate `pods/exec` per container).
    container: Option<String>,
}
```

- **Classification** (`mutation_target()`): `ReadOnly` — a `SelfSubjectAccessReview` never
  writes cluster state.
- **Section** (`section()`): `section_keys::workloads_pods()`, same as the other pod-exec
  commands.

### `BackendEvent`/`ResourceReadEvent::PodExecAccessChecked` (new event)

```rust
PodExecAccessChecked {
    namespace: String,
    name: String,
    container: Option<String>,
    allowed: bool,
    /// Verbatim reason from the `SelfSubjectAccessReview` status, or the review call's
    /// own error, when present.
    reason: Option<String>,
}
```

Emitted once per `CheckPodExecAccess`, on the pod's namespaced route (`exec_route`-style: same
`Route` helper `src/backend/exec.rs` already uses for `PodExecReady`/`PodExecOutputChunk`), so
the frontend receives it as an ordinary namespaced resource-read event scoped to the current
context/namespace — no new stream/correlation plumbing.

### `MutationTarget` reclassification (no new variant)

`StartPodExec` moves from the `ReadOnly` match arm to the `Namespaced { namespaces: vec![namespace] }`
arm in `BackendCmdV1::mutation_target()`. `SendPodExecInput`, `ResizePodExec`, `StopPodExec`,
and the new `CheckPodExecAccess` remain `ReadOnly`.

## Frontend state additions

### `PodTerminalTab` phase union (extended)

```ts
type Phase =
  | 'picking'     // existing: choosing a container (>1 container)
  | 'checking'    // NEW: CheckPodExecAccess sent, awaiting PodExecAccessChecked
  | 'denied'      // NEW: access check returned allowed: false
  | 'confirming'  // NEW: access check returned allowed: true; ConfirmDialog is open
  | 'attaching'   // existing, unchanged
  | 'active'      // existing, unchanged
  | 'ended'        // existing, unchanged
  | 'error'       // existing, unchanged
```

### Store: exec-access-check slice (new, transient — not persisted)

Keyed by `${namespace}/${name}/${container ?? ''}`, holding the most recent
`PodExecAccessChecked` payload (`allowed`, `reason`, and a monotonic `rev` counter for the
component's `useEffect` dependency, mirroring the existing `ExecStreamState.rev` pattern used
for stream chunks). Entries are ephemeral UI state, not part of `ExecStreamState`'s stream
buffer — a `CheckPodExecAccess` never opens a stream.

## State transitions (`PodTerminalTab`)

```
picking --(container resolved)--> checking
checking --(PodExecAccessChecked allowed:false)--> denied
checking --(PodExecAccessChecked allowed:true)--> confirming
checking --(pod/namespace changes before result arrives)--> picking (check superseded/ignored)
confirming --(operator declines/dismisses)--> picking  (if >1 container) | idle-not-started (if 1)
confirming --(operator confirms)--> attaching            [existing StartPodExec flow, unchanged]
denied --(operator changes pod or, if >1 container, container)--> checking
```

No transition from `confirming` or `denied` ever sends `StartPodExec`; only an explicit
confirm does.
