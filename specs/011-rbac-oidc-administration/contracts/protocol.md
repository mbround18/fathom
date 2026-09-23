# Contract: New `BackendCmdV1` / `BackendEventV1` surface for 011-rbac-oidc-administration

This feature's UI-to-backend contract is the versioned protocol in `src/protocol/v1/`. Existing
commands reused unchanged: `RefreshRoles`, `SelectRole`, `DeleteRole`, `RefreshClusterRoles`,
`SelectClusterRole`, `DeleteClusterRole`, `RefreshRoleBindings`, `SelectRoleBinding`,
`DeleteRoleBinding`, `RefreshClusterRoleBindings`, `SelectClusterRoleBinding`,
`DeleteClusterRoleBinding`, `ApplyResourceYaml` (for RoleBinding/ClusterRoleBinding create+edit).

## New commands

| Command | Mutation target | Section | Purpose |
|---|---|---|---|
| `RefreshRbacSelfAccess { checks: Vec<RbacAccessCheck> }` | ReadOnly | `access/rolebindings` / `access/clusterrolebindings` (whichever the caller is gating) | FR-007 gating |
| `ResolveNamespaceAccess { subject_kind, subject_name, subject_namespace }` | ReadOnly | `access/namespace-access` (new) | FR-011/FR-012 |
| `RefreshKubeadmDetection` | ReadOnly | `access/oidc-config` (new) | FR-009 scenario 1 |

## New events

| Event | Carries | Notes |
|---|---|---|
| `RbacSelfAccessUpdated { results: Vec<RbacAccessResult> }` | per-check allowed/reason | one result per input check, same order |
| `NamespaceAccessResolved(NamespaceAccessResolved)` | full resolved-access payload | `found_any: false` when no bindings match |
| `KubeadmDetectionUpdated(KubeadmDetectionUpdated)` | `{ detected, checked }` | never an error event; unreadable/absent ConfigMap => `checked: false` |

## Error handling contract

- `ResolveNamespaceAccess` and `RefreshRbacSelfAccess` never fail the whole request over a
  partial-read problem: a namespace/scope the operator's identity can't list is added to
  `degraded_scopes` (namespace-access) or the specific check gets `allowed: false, reason: Some(<verbatim API error>)`
  (self-access) — this matches FR-001 scenario 3 (verbatim errors, not silent omission) applied
  to the two new read paths.
- `ApplyResourceYaml` conflict/not-found/other-error wire contract is unchanged (see
  research.md) — reused verbatim for RoleBinding/ClusterRoleBinding writes.

## Backward compatibility

All additions are new enum variants; `MutationTarget::mutation_target()` and `section()` matches
in `src/protocol/v1/mutation.rs` are exhaustive with no wildcard arm, so the compiler forces
arms to be added for each new variant (this is the existing drift-prevention mechanism, not new
process). `tests/protocol_serde.rs`'s `*_is_exhaustive` matches serve the same purpose for
serde coverage.
