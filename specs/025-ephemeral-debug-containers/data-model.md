# Phase 1 Data Model: Ephemeral Debug Containers

## New / changed types (`src/protocol/v1/`)

### `BackendCmdV1::AddEphemeralContainer` (cmd.rs, new variant)

| Field | Type | Notes |
|---|---|---|
| `namespace` | `String` | Pod's namespace. |
| `pod` | `String` | Pod name. |
| `target_container` | `Option<String>` | Container whose process namespace to share (`spec.ephemeralContainers[].targetContainerName`). `None` = backend auto-selects when the pod has exactly one container; otherwise a validation error is reported back via `CommandFinished`. |
| `image` | `String` | Debug image reference, free text (spec FR-002). Must be non-empty; UI enforces this before submit (FR-013), backend re-validates. |
| `ephemeral_container_name` | `Option<String>` | Name for the new ephemeral container. `None` = backend generates one (e.g. `debugger-<suffix>`). |

Classified in `mutation.rs` as `MutationTarget::Namespaced { namespaces: vec![namespace] }`.

Outcome reporting: no new event variant. Success/failure surfaces through the existing
`BackendEventV1::CommandFinished { request_id, outcome: ActionOutcome, .. }` path used by all
other mutating commands, so RBAC-denied/duplicate-name/invalid-image/not-found errors reach the
UI verbatim per constitution principle I.

### `ContainerLifecycle` (pods.rs, additive field)

| Field | Type | Notes |
|---|---|---|
| `is_ephemeral` | `bool` | **New.** `true` for entries sourced from `status.ephemeralContainerStatuses[]`, `false` for entries from `status.containerStatuses[]`/`status.initContainerStatuses[]` (existing behavior, unchanged). |

All other `ContainerLifecycle` fields (`name`, `is_init`, `image`, `ready`, `state`,
`restart_count`, `last_state`, `probes`, `hooks`) are populated the same way for ephemeral
container entries as for regular ones, using whatever subset of that data
`status.ephemeralContainerStatuses[]` actually reports (ephemeral containers have no probes/hooks
in their spec, so those two fields are simply empty vectors for ephemeral entries — no schema
change needed, just naturally-empty data).

`is_init` is always `false` for ephemeral entries (ephemeral containers are never init
containers).

## Validation rules

- `image` MUST be non-empty (UI-side: submit disabled; backend-side: command rejected with a
  validation error surfaced through `CommandFinished` before any API call is attempted).
- `target_container`, when the pod has more than one existing (non-ephemeral) container, MUST be
  one of those container names; the UI selector only offers valid choices, so a mismatch here
  indicates a stale pod view and is surfaced as a normal command error.
- No uniqueness pre-check is performed client-side for `ephemeral_container_name` — Kubernetes
  itself rejects a duplicate name, and that rejection is surfaced verbatim (edge case in spec).

## State transitions

Ephemeral container status transitions are entirely server-driven and already modeled by
`ContainerState.phase` (`"waiting"` → `"running"` → `"terminated"`, optionally with `reason` for
`ImagePullBackOff`-style waiting states). No new state machine is introduced — this feature only
adds a way to populate a `ContainerLifecycle` entry with `is_ephemeral: true` and to originate the
API call that causes Kubernetes to eventually report one.

## Relationships

```text
Pod (existing PodSummary/PodDetails)
 └─ containers_lifecycle: Vec<ContainerLifecycle>   (existing, additive)
     ├─ regular container entries   (is_ephemeral = false, unchanged)
     └─ ephemeral container entries (is_ephemeral = true, new — sourced from
        status.ephemeralContainerStatuses[])
```

No new top-level entity is introduced. The "Ephemeral Debug Container Request" described in the
spec's Key Entities section is transient UI form state (`ui/src/resources/pods.tsx` /
`ActionDialogHost.tsx` local state) — it is not persisted and has no protocol representation
beyond the `AddEphemeralContainer` command fields it is turned into on submit.
