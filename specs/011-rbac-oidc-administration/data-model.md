# Phase 1 Data Model: RBAC/OIDC Administration

## Existing entities (read-only, unchanged)

- `RoleSummary`/`RoleDetails`, `ClusterRoleSummary`/`ClusterRoleDetails`,
  `RoleBindingSummary`/`RoleBindingDetails`, `ClusterRoleBindingSummary`/
  `ClusterRoleBindingDetails`, `SubjectSummary`, `PolicyRuleSummary` — all already defined in
  `src/protocol/v1/` and generated to `ui/src/protocol/generated/`. See research.md; no field
  changes required. `role_ref` on binding summaries/details is a `"Kind/name"` string (e.g.
  `"ClusterRole/view"` or `"Role/pod-reader"`) already resolvable for dangling-reference display
  by checking membership against the already-fetched Role/ClusterRole list for the frontend, or
  a backend-computed `bool` addition (`role_ref_exists`? — see Open Question below, resolved:
  compute client-side, no field addition, since the frontend already fetches both lists for the
  same section group and dangling-detection is a pure join, avoiding backend churn).

## New protocol types

### `RbacAccessCheck` (command input)
```rust
struct RbacAccessCheck {
    verb: String,            // "create" | "update" | "patch" | "delete"
    resource: String,         // "rolebindings" | "clusterrolebindings"
    namespace: Option<String>, // Some(ns) for RoleBinding checks, None for cluster-scoped
}
```

### `RbacAccessResult` (event output)
```rust
struct RbacAccessResult {
    check: RbacAccessCheck,
    allowed: bool,
    reason: Option<String>, // verbatim reason string from the SelfSubjectAccessReview status, if any
}
```
Validation: none (derived, read-only). Round-trips via `tests/protocol_serde.rs`.

### `NamespaceAccessEntry`
```rust
struct NamespaceAccessEntry {
    namespace: String,                 // ignored/absent when cluster_wide
    verbs: Vec<String>,                // deduplicated, sorted
    resources: Vec<String>,            // deduplicated, sorted, "group/resource" form
    via: Vec<String>,                  // binding names contributing this access, e.g. "RoleBinding/foo"
}
```

### `NamespaceAccessResolved` (event)
```rust
struct NamespaceAccessResolved {
    subject_kind: String,   // "Group" | "User" | "ServiceAccount"
    subject_name: String,
    subject_namespace: Option<String>, // required for ServiceAccount, ignored otherwise
    cluster_wide: Vec<NamespaceAccessEntry>,   // entries from ClusterRoleBinding matches (namespace field unused/empty)
    per_namespace: Vec<NamespaceAccessEntry>,  // entries from RoleBinding matches, one per namespace
    dangling_bindings: Vec<String>,     // "RoleBinding/ns/name" or "ClusterRoleBinding/name" whose roleRef is missing
    degraded_scopes: Vec<String>,       // namespaces/scopes the operator's identity could not list (FR partial-read edge case)
    found_any: bool,
}
```
Validation: subject_name MUST NOT be empty (rejected client-side before sending the command,
same validation class as binding subject names per FR-014's sibling rule — this is a read
request, not a write, so rejection is a disabled "Look up" button plus inline message, not a
protocol-level error).

### `KubeadmDetectionUpdated` (event)
```rust
struct KubeadmDetectionUpdated {
    detected: bool,          // true only if kube-system/kubeadm-config ConfigMap was readable and present
    checked: bool,           // false if the read itself failed/was forbidden (distinguishes "no" from "unknown")
}
```

## New commands (`BackendCmdV1`, all `MutationTarget::ReadOnly`)

```rust
RefreshRbacSelfAccess { checks: Vec<RbacAccessCheck> },
ResolveNamespaceAccess { subject_kind: String, subject_name: String, subject_namespace: Option<String> },
RefreshKubeadmDetection,
```

## New events (`BackendEventV1`)

```rust
RbacSelfAccessUpdated { results: Vec<RbacAccessResult> },     // ResourceReadEvent-style, routed to access_* sections
NamespaceAccessResolved(NamespaceAccessResolved),               // own event, section = access/namespace-access (new SectionKey)
KubeadmDetectionUpdated(KubeadmDetectionUpdated),                // own event, section = access/oidc-config (new SectionKey)
```

## Section keys (new)

- `section_keys::access_namespace_access()` → wire string `"access/namespace-access"`
- `section_keys::access_oidc_config()` → wire string `"access/oidc-config"`

Both registered under the existing `NavGroup::AccessControl` group in
`src/sections/mod.rs::access_control_sections()`, `ScopeKind::Cluster` (both are cluster-wide
tools, not per-namespace resource lists).

## Frontend-only entities (no protocol representation)

- `OidcGeneratorInput { issuerUrl, clientId, audience, usernameClaim, groupsClaim, extraScopes: string[] }`
- `OidcGeneratorOutput { kubeadmFlags: string, providerInstructions: { provider: string; text: string }[] }`
  — pure function of `OidcGeneratorInput`, computed and rendered client-side only, never sent
  over the protocol (FR-008/FR-010).
- `RoleBindingFormState { kind: 'RoleBinding' | 'ClusterRoleBinding', namespace?, name, roleRef: { kind: 'Role'|'ClusterRole', name }, subjects: SubjectSummary[] }`
  — the structured form backing the YAML the frontend generates for `ApplyResourceYaml`.
