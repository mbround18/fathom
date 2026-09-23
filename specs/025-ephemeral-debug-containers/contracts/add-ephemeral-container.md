# Contract: `AddEphemeralContainer`

Protocol location: `src/protocol/v1/cmd.rs` (`BackendCmdV1` variant), classified in
`src/protocol/v1/mutation.rs`.

## Command (UI → backend)

```jsonc
{
  "type": "AddEphemeralContainer",
  "data": {
    "namespace": "default",
    "pod": "checkout-7f9b6c9d8-abcde",
    "target_container": "checkout",       // nullable
    "image": "nicolaka/netshoot:latest",
    "ephemeral_container_name": null       // nullable; backend generates if omitted
  }
}
```

Preconditions (enforced client-side before send, per FR-013):
- `image` is non-empty.
- `target_container` is set, OR the pod has exactly one existing container (auto-selected).
- The user has confirmed through `ConfirmDialog` (FR-004/005/006).

## Response (backend → UI)

No new event variant. Reuses the existing generic mutation-outcome event:

```jsonc
{
  "type": "CommandFinished",
  "data": {
    "request_id": "...",
    "outcome": { "Ok": null }              // or { "Err": "<verbatim k8s API error>" }
  }
}
```

On success, the pod's next `PodDetails`/`containers_lifecycle` refresh (existing refresh/watch
path — no new event) includes a new entry with `is_ephemeral: true`, `name` equal to the resolved
ephemeral container name, and a `state` reflecting the live Kubernetes status
(`waiting`/`running`/`terminated`).

## Mutation classification contract

`AddEphemeralContainer { namespace, .. }.mutation_target()` MUST return
`MutationTarget::Namespaced { namespaces: vec![namespace] }`, so that:
- `writes_cluster()` returns `true` (routes through the UI confirm-gate).
- Any sandbox/guard consumer of `MutationTarget` (e.g. `fathom-devbridge`) treats it as a
  namespaced cluster write like `DeletePod`/`ScaleDeployment`.

## UI contract (`ui/src/resources/pods.tsx` action descriptor)

- `destructive: true` (irreversible framing, per spec FR-005).
- `detail` text MUST include: target pod name, chosen target container name, debug image name,
  and the fixed sentence: *"Ephemeral containers cannot be removed once added and will persist
  for the life of the pod."*
- Submit control MUST be disabled while `image` is empty or `target_container` is required but
  unset (FR-013).

## Non-contract (explicitly out of scope)

- No `RemoveEphemeralContainer` command exists or will exist (FR-009) — this is a one-way
  contract by design, matching the real Kubernetes API surface.
- No changes to the `StartPodLogs`/`StartPodExec` command contracts — an ephemeral container's
  name is passed through their existing `container: Option<String>` field unchanged.
