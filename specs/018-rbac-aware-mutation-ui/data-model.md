# Phase 1 Data Model: Proactive RBAC-Aware Mutation UI

No new persisted entities or database tables. This feature introduces one new
in-flight (non-persisted) request/response pair and reuses spec 011's existing
result type. All state lives for the life of the app session in the Zustand
store (`rbacAccess.selfAccess`).

## Action Access Check (existing type, reused)

Source: `src/protocol/v1/rbac_access.rs` (spec 011, unchanged).

`RbacAccessCheck`:
| Field       | Type             | Notes |
|-------------|------------------|-------|
| `verb`      | `String`         | Kubernetes verb, e.g. `"update"`, `"delete"`, `"patch"`. |
| `resource`  | `String`         | Plural resource name, e.g. `"nodes"`, `"secrets"`, `"rolebindings"`. |
| `namespace` | `Option<String>` | `None` for cluster-scoped resources (nodes, ClusterRoleBindings). |

`RbacAccessResult`:
| Field    | Type             | Notes |
|----------|------------------|-------|
| `check`  | `RbacAccessCheck`| Echo of the request, used as the merge key. |
| `allowed`| `bool`           | SelfSubjectAccessReview's `status.allowed`. |
| `reason` | `Option<String>` | `status.reason` or `status.evaluation_error`, shown in the tooltip. |

**Merge key**: `(verb, resource, namespace)` — used both to correlate a result
back to the check that produced it and to merge into the store slice without
clobbering concurrently-in-flight checks from other panels (see research.md
Decision 4).

## New: Resource Mutation Access Check (YAML apply/create only)

Source: `src/protocol/v1/cmd.rs` / `event.rs` (new in this feature).

`CheckResourceMutationAccess` (command):
| Field         | Type             | Notes |
|---------------|------------------|-------|
| `api_version` | `String`         | As parsed from the YAML document, e.g. `"apps/v1"`. |
| `kind`        | `String`         | As parsed from the YAML document, e.g. `"Deployment"`. |
| `namespace`   | `Option<String>` | Target namespace, or `None` for cluster-scoped kinds. |
| `verb`        | `String`         | `"create"` or `"update"`, per whether the target object already exists. |

`ResourceMutationAccessChecked` (event):
| Field    | Type              | Notes |
|----------|-------------------|-------|
| `result` | `RbacAccessResult`| Same shape as spec 011's result, with `check.resource`/`check.group`-equivalent populated from cluster discovery of `api_version`+`kind` (not guessed client-side). |

Validation rule: if discovery fails to resolve `api_version`+`kind` to a known
API resource, the handler emits `allowed: false` with a reason describing the
discovery failure (e.g. unknown CRD) rather than erroring the whole command —
this is a normal, expected "can't tell, so don't claim it's fine" case for a
YAML document that doesn't yet match a registered API (FR-006's fallback
still applies at the frontend: unknown/undetermined never renders a false
"denied" tooltip, only a genuine denial does).

## Mutation Control Binding (frontend-only concept, not a wire type)

Conceptual mapping table (implemented as call-site wiring, not a shared data
structure) between each gated control and the check(s) it uses:

| Surface | Control | Verb | Resource | Namespace |
|---|---|---|---|---|
| Node actions | Cordon/Uncordon/Drain/Taint add/remove | `update` | `nodes` | (cluster-scoped, `None`) |
| YAML apply/create | Apply button | `create` or `update` (per existing-object check) | *(discovered)* | target namespace or `None` |
| Helm | Upgrade / Rollback | `update` | `secrets` | release namespace |
| Helm | Uninstall | `delete` | `secrets` | release namespace |
| RBAC admin | Create/Edit RoleBinding | `create`/`update` | `rolebindings` | binding's namespace |
| RBAC admin | Create/Edit ClusterRoleBinding | `create`/`update` | `clusterrolebindings` | (cluster-scoped, `None`) |
| RBAC admin | Delete RoleBinding | `delete` | `rolebindings` | binding's namespace |
| RBAC admin | Delete ClusterRoleBinding | `delete` | `clusterrolebindings` | (cluster-scoped, `None`) |

Each row becomes one `useMutationAccess([check])` call (or a shared call when
a panel has multiple related actions against the same resource/namespace,
e.g. all five node actions share one `nodes` check).

## `ActionDescriptor` extension

`ui/src/resources/types.ts` — one new optional field, additive only:

```ts
accessCheck?: {
  verb: string;
  resource: string;
  namespace?: string;
};
```

When present, `ActionMenu.tsx` resolves it through `useMutationAccess` and
disables the menu item + shows the tooltip reason when `allowed === false`;
absent (the default for the other ~30 resource kinds' existing actions)
preserves current behavior exactly — no change for anything not explicitly
wired in this feature.
