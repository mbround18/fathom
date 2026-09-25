# Protocol Contract: 018-rbac-aware-mutation-ui

All additions are backward compatible within protocol v1 and read-only (classified `MutationTarget::ReadOnly`).

## Changed: `RbacAccessCheck` gains `group`

The spec-011 handler hardcodes `group: "rbac.authorization.k8s.io"` in its `SelfSubjectAccessReview`,
which is wrong for `nodes` and `secrets` (core group ""). Add:

```rust
#[serde(default)]
pub group: Option<String>,   // None => core API group ""
```

The handler uses `check.group.unwrap_or_default()`. Existing callers (`RoleBindingForm`) now pass
`"rbac.authorization.k8s.io"` explicitly (through the shared hook). The merge key on the frontend is
`(verb, group, resource, namespace)`.

## New command: `CheckResourceMutationAccess`

```rust
CheckResourceMutationAccess { api_version: String, kind: String, namespace: Option<String>, verb: String }
```

Handler: discover the `ApiResource` for `api_version`/`kind` via `kube::discovery::oneshot::pinned_kind`
(shared helper in `apply.rs`), run one `SelfSubjectAccessReview` with the discovered group/plural, emit
`ResourceMutationAccessChecked`. Discovery failure yields `allowed: false` with the reason (never a command error).

## New event: `ResourceMutationAccessChecked`

`ResourceReadEvent::ResourceMutationAccessChecked { api_version, kind, result: RbacAccessResult }`

The frontend also merges `result` into `rbacAccess.selfAccess` (same slice as `RbacSelfAccessUpdated`).
The `api_version`/`kind` echo lets the YAML apply UI look up its result.
