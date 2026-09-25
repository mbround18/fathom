# Protocol Contract: Pod Exec Session Gate

Extends the existing versioned protocol in `src/protocol/v1/`. See `data-model.md` for full
field definitions. This file states the wire contract and its invariants.

## Command: `CheckPodExecAccess`

```rust
BackendCmdV1::CheckPodExecAccess { namespace: String, name: String, container: Option<String> }
```

- **Direction**: UI → backend.
- **Preconditions**: none (safe to call for any pod the operator can see; it is itself a
  read-only self-access check).
- **Effect**: Issues one `SelfSubjectAccessReview` (`resource: "pods"`,
  `subresource: "exec"`, `verb: "create"`, `group: ""`, scoped to `namespace`/`name`) against
  the active cluster context. Never mutates cluster or app state.
- **Response**: Exactly one `PodExecAccessChecked` event (success or the review call erroring
  — an erroring review is reported as `allowed: false` with the error's message as `reason`,
  mirroring `handle_refresh_rbac_self_access`'s existing error handling).
- **Idempotent**: yes; safe to call repeatedly (e.g. if the operator switches containers).

## Event: `PodExecAccessChecked`

```rust
BackendEvent::ResourceRead(ResourceReadEvent::PodExecAccessChecked {
    namespace: String,
    name: String,
    container: Option<String>,
    allowed: bool,
    reason: Option<String>,
})
```

- **Direction**: backend → UI.
- **Invariant**: `allowed: false` MUST always carry a human-readable `reason` when the
  underlying API call failed outright (network/auth error) or the review explicitly denied
  access with a server-provided reason; `reason` MAY be `None` when the server simply returned
  `allowed: false` with no explanation (a valid, common case for a plain RBAC deny).

## Reclassification: `StartPodExec`

`BackendCmdV1::mutation_target()` for `StartPodExec { namespace, .. }` changes from
`MutationTarget::ReadOnly` to `MutationTarget::Namespaced { namespaces: vec![namespace] }`.

- **Consumers affected**:
  - `tools/devbridge/src/guard.rs`: `StartPodExec` now requires the target namespace to be
    allow-listed for writes through the dev bridge, exactly like `DeletePod`/`ScaleDeployment`/etc.
  - `src/backend.rs::finished_route()`: `StartPodExec`'s `CommandFinished` event now routes to
    the target namespace instead of the namespace-agnostic fallback route.
- **Unaffected**: `SendPodExecInput`, `ResizePodExec`, `StopPodExec`, `CheckPodExecAccess` all
  remain `ReadOnly` — only the act of opening a session is gated.

## UI contract: confirmation before attach

`PodTerminalTab` MUST NOT send `StartPodExec` except as the direct result of the operator
activating a `ConfirmDialog`'s confirm action, and MUST NOT show that `ConfirmDialog` unless
the most recent `PodExecAccessChecked` for the current namespace/name/container reported
`allowed: true`. This is a client-side contract (enforced by the component's state machine in
`data-model.md`), not a new wire message — the wire-level gate is `StartPodExec`'s
`MutationTarget::Namespaced` reclassification above, which is the enforceable boundary for any
client, current or future.
