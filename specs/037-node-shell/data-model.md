# Data Model: Node Shell (transient except settings)

## NodeShellSettings (persisted, per context, `~/.fathom/`)
| Field | Type | Default | Notes |
|---|---|---|---|
| image | string | `busybox:stable` | non-empty, trimmed |
| namespace | string | `default` | valid DNS label |
| image_pull_secret | string? | null | name only; contents never read |
| deadline_seconds | u32 | 14400 | clamped 300..86400 |

## NodeShellSession (in-memory, singleton)
| Field | Type | Notes |
|---|---|---|
| session_id | string | generation token; stale events discarded |
| context | string | context that owns the client used for delete |
| node | string | |
| namespace / pod_name | string | `fathom-node-shell-<node-hash>-<rand5>` (<=63 chars) |
| image | string | effective image |
| phase | enum | Creating, Pending, Running, Attached, Terminating, Removed, Failed |
| detail | string? | verbatim reason (pod condition or API error) |
| leftover | bool | true when delete failed |

Transitions: Creating -> Pending -> Running -> Attached -> Terminating -> Removed. Any -> Failed (then Terminating -> Removed, or leftover). Stop from any non-terminal phase goes to Terminating.

## NodeShellAccessResult
`checks: [{ verb, resource, subresource?, namespace?, outcome: Allowed | Denied(reason) | Error(msg) }]`; overall = all Allowed.

## OrphanedNodeShellPod
`{ namespace, name, node, phase, created_at }` found via label `fathom.io/node-shell=true`.

## Pod labels/annotations
Labels: `app.kubernetes.io/managed-by=fathom`, `fathom.io/node-shell=true`, `fathom.io/node=<node>`. Annotation: `fathom.io/created-at`.
