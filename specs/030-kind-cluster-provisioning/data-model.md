# Data Model: In-App Local Kind Cluster Provisioning

## KindPrerequisites

Reported state of the host's readiness for this feature. Backend-computed, read-only from the
frontend's perspective.

| Field | Type | Notes |
|---|---|---|
| `kind_installed` | bool | `kind` binary found on `PATH`. |
| `kind_version` | `Option<String>` | Parsed from `kind version` when installed. |
| `runtime_installed` | bool | Docker or Podman binary found on `PATH`. |
| `runtime_kind` | `Option<"docker" \| "podman">` | Which runtime was detected, if any. |
| `runtime_running` | bool | The detected runtime responded successfully to a ping (`docker info` / `podman info`). Meaningless (`false`) if `runtime_installed` is false. |
| `ready` | bool | Derived: `kind_installed && runtime_installed && runtime_running`. Computed backend-side so the frontend never re-derives readiness logic. |

State transitions: none — this is a point-in-time snapshot re-fetched on demand
(`RefreshKindPrerequisites`), never partially updated.

## KindClusterSummary

One row in the local cluster list.

| Field | Type | Notes |
|---|---|---|
| `name` | String | DNS-1123-label-safe (see Validation Rules). Unique among local kind clusters. |
| `status` | `"ready" \| "in_progress" \| "error"` | `ready` for clusters returned by `kind get clusters` outside of any tracked operation; `in_progress`/`error` only apply while a `KindClusterOperation` for that name is tracked in backend memory (not persisted — resets to `ready`/absent after a restart, reconciled by the next list refresh). |
| `profile` | `"bare" \| "custom_yaml"` | What the cluster was created with, when known (best-effort; a cluster created outside the app or before this field existed reports `None`/omitted). |
| `context_name` | String | The kubeconfig context name kind registers for this cluster (`kind-<name>` by kind's own convention), used to cross-reference against spec 012's context list. |

## KindClusterOperation

A tracked, in-flight create or delete action. Exists only in backend memory for the duration of
the operation; not persisted to disk (Constitution VI does not apply to this ephemeral,
in-memory tracking — it is not `~/.fathom/` state).

| Field | Type | Notes |
|---|---|---|
| `request_id` | String | Correlates `BackendCmdV1` → `StreamEvent`s → terminal event, mirroring `DrainNode`'s `request_id`. |
| `cluster_name` | String | Target cluster name (validated). |
| `kind` | `"create" \| "delete"` | Operation type. |
| `stage` | String | Latest human-readable stage line (e.g. "Preparing nodes 📦"), sourced from `kind`'s own stderr progress output for create; a small fixed set of stages for delete ("Deleting cluster..."). |

**Concurrency rule** (FR-016): the backend MUST reject a new `CreateKindCluster` or
`DeleteKindCluster` command whose `cluster_name` matches an already-tracked
`KindClusterOperation`, returning an error immediately (surfaced verbatim per Principle I)
rather than queuing or silently ignoring it.

## KindClusterProfile

A named starting configuration.

| Field | Type | Notes |
|---|---|---|
| `id` | `"bare" \| "custom_yaml"` | v1 only ships these two; extensible enum for future profiles (see research.md R6). |
| `display_name` | String | e.g. "Bare single-node", "Custom (paste kind-config YAML)". |
| `description` | String | Shown in the pre-creation summary (FR-008). |

## Validation Rules

- **Cluster name**: 1-63 characters; lowercase ASCII letters, digits, and `-`; must start and
  end with an alphanumeric character (RFC 1123 DNS label, matching what `kind`/Kubernetes
  itself requires of a cluster name that becomes part of a context/node name). Enforced in Rust
  as the source of truth; mirrored in the frontend for immediate inline feedback. Reserved
  values: none beyond the syntactic rule and the "must not collide with an existing local kind
  cluster name" rule (FR-007), checked against the latest `KindClusterSummary` list at the
  moment of submission (re-validated backend-side, not just cached from the last list fetch).
- **Custom YAML profile**: when `profile = custom_yaml`, the pasted content MUST parse as valid
  YAML before the confirmation step is reachable; semantic validation of its contents is
  delegated to `kind` itself at creation time (its own error, if any, is surfaced verbatim).

## Relationship to Spec 012 (Context Federation)

`KindClusterSummary.context_name` is not a new identity system — it is exactly the context name
that appears in spec 012's existing `contexts`/`FederationContextSwitcher` state after
`kind create cluster` merges it into the shared kubeconfig and Fathom's existing
`RefreshContexts` path re-reads that file (see research.md R4). No new entity mirrors or
duplicates spec 012's `Context`/`ContextStatus` types; `KindClusterSummary` and spec 012's
context list are cross-referenced by name at render time in the frontend only (e.g. to show a
"currently selected" badge on the row whose `context_name` matches
`session.activeContext`), never merged into one persisted structure.
