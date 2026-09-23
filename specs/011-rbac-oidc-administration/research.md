# Phase 0 Research: RBAC/OIDC Administration

All unknowns from the spec were resolvable directly from the existing codebase; no external
research was required.

## Decision: Reuse `ApplyResourceYaml` for RoleBinding/ClusterRoleBinding create + edit

- **Decision**: Do not add a bespoke `CreateRoleBinding`/`UpdateRoleBinding` command. The
  frontend builds the target YAML (from a structured subject/roleRef form) and sends it through
  the existing generic `ApplyResourceYaml { api_version, kind, namespace, name, yaml,
  expected_resource_version }` command, exactly as 003's YAML editor does for any kind.
- **Rationale**: `src/backend/apply.rs` already does GVK discovery via
  `kube::discovery::oneshot::pinned_kind` (works for any built-in kind including
  `rbac.authorization.k8s.io/v1` `RoleBinding`/`ClusterRoleBinding`), stamps
  `expected_resource_version` for optimistic-concurrency conflict detection, and classifies
  errors into `Conflict`/`NotFound`/`Other` with a verbatim-message wire contract the frontend
  already parses. Reusing it satisfies FR-003/FR-004/FR-006 with zero new backend mutation code,
  and structurally guarantees Constitution Principle I parity with 003 rather than risking drift
  from a second implementation.
- **Alternatives considered**: A typed `CreateRoleBinding`/`UpdateRoleBinding` command with
  structured `roleRef`/`subjects` fields — rejected because it would duplicate the
  diff/apply/conflict machinery `apply.rs` already has, doubling the surface area to keep
  RBAC-escalation-error passthrough (FR-006) and conflict handling (FR-004) correct, for no
  behavioral benefit over building the YAML client-side and applying it generically. The
  structured **form** the operator fills in (role, namespace, subjects) still exists in the UI;
  it is just a YAML-generating front-end for the existing generic apply command, not a new
  protocol shape.

## Decision: New read-only command for self-permission gating (FR-007)

- **Decision**: Add `RefreshRbacSelfAccess { checks: Vec<RbacAccessCheck> }` →
  `RbacSelfAccessUpdated { results: Vec<RbacAccessResult> }`, backed by
  `SelfSubjectAccessReview` (`authorization.k8s.io/v1`, already available via `k8s-openapi`
  `v1_34` feature already enabled in `Cargo.toml`). One review per (verb, resource,
  namespace-or-cluster) tuple needed to gate create/edit/delete of RoleBinding and
  ClusterRoleBinding.
- **Rationale**: `SelfSubjectAccessReview` is the Kubernetes-native, RBAC-aware way to ask "can I
  do X" without guessing from locally cached Role/ClusterRoleBinding data (which could be stale
  or incomplete if the operator's own bindings aren't fully visible to them). This mirrors
  spec's explicit requirement that FR-007 be "via the Kubernetes API", and needs no new
  dependency.
- **Alternatives considered**: Client-side inference from the namespace-access resolver (Story 4)
  — rejected per spec's edge cases: the operator may lack read access to their own bindings even
  though they can write, so a live API-server check is required, not a derived one.

## Decision: Namespace-access resolution computed from cached list data, no new cluster reads

- **Decision**: `ResolveNamespaceAccess { subject_kind, subject_name, subject_namespace }` →
  `NamespaceAccessResolved { ... }` runs entirely against the Role/ClusterRole/RoleBinding/
  ClusterRoleBinding objects the backend already lists for the Access Control sections (it
  triggers the same `list_all` calls `refresh_roles`/`refresh_role_bindings`/etc. use, scoped to
  what the operator's own identity can already read — consistent with FR-011's "computed
  entirely from ... objects already readable by the operator's identity").
- **Rationale**: Avoids a second, parallel data-fetch path; reuses exactly the same
  Kubernetes-visible data already surfaced in Stories 1-3, keeping "same data, same partial/
  degraded-read behavior" (edge case: missing read permission on Roles/ClusterRoles must degrade
  gracefully, matching FR-001's acceptance scenario 3 requirement to surface exactly which
  scopes were inaccessible).
- **Alternatives considered**: A server-side `kubectl auth can-i --list`-style bulk review —
  rejected: `SelfSubjectRulesReview` only covers the caller's own identity, not an arbitrary
  subject being looked up, so it cannot answer "what can *this OIDC group* do" — only the direct
  Role/ClusterRole/RoleBinding/ClusterRoleBinding graph walk (matching `roleRef` + `subjects`)
  can, which is exactly what FR-011 specifies.

## Decision: kubeadm-cluster detection is a best-effort read, not a blocking dependency

- **Decision**: `RefreshKubeadmDetection` → `KubeadmDetectionUpdated { detected: bool }` reads
  `kube-system/kubeadm-config` ConfigMap; any error (including "not found" or "forbidden") maps
  to `detected: false` with no error surfaced (this is advisory UI framing only, not a
  correctness-critical read) — never blocks the OIDC generator from working (FR-009's flag
  generation works from operator-entered fields alone).
- **Rationale**: Matches spec's Story 5 acceptance scenario 1 ("where determinable"); a managed
  cluster or an operator without ConfigMap-read access simply sees "could not determine cluster
  type" rather than an error, and generation of the flag set/instructions is unaffected either
  way.

## Decision: OIDC config generator is 100% frontend, no protocol write at all

- **Decision**: Given FR-008/FR-010, the flag-set and provider-instruction generation is pure
  TypeScript (issuer/client-id/audience/claims → `kube-apiserver` flags string + provider text
  blocks), rendered in a visually distinct "apply this yourself" panel, with a client-side
  copy/export-to-file action (Tauri's existing file-save dialog, already used elsewhere in the
  app for exports — confirm exact API at implementation time, e.g. `@tauri-apps/plugin-dialog`
  if already a frontend dependency).
- **Rationale**: No cluster interaction should occur per FR-008; keeping this entirely
  client-side is the simplest way to guarantee that invariant (there's no command to accidentally
  wire to a cluster write).
